use lazy_regex::Regex;
use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_str::CompactStr;
use rustc_hash::{FxHashMap, FxHashSet};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::{tailwind_classes, tailwind_literal},
};

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EnforceCanonicalClassesConfig {
    collapse: bool,
    logical: bool,
    root_font_size: Option<f64>,
    ignore: Vec<CompactStr>,
}

impl Default for EnforceCanonicalClassesConfig {
    fn default() -> Self {
        Self { collapse: true, logical: true, root_font_size: None, ignore: Vec::new() }
    }
}

#[derive(Debug, Default, Clone)]
struct EnforceCanonicalClassesOptions {
    collapse: bool,
    logical: bool,
    root_font_size: Option<f64>,
    ignore: Vec<Regex>,
}

#[derive(Debug, Default, Clone)]
pub struct EnforceCanonicalClasses(Box<EnforceCanonicalClassesOptions>);

#[derive(Debug, Deserialize)]
struct CanonicalClass {
    input: Vec<String>,
    output: String,
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Applies canonical candidate suggestions from Tailwind v4's design system.
    EnforceCanonicalClasses,
    better_tailwindcss,
    style,
    fix,
    config = EnforceCanonicalClassesConfig,
    version = "4.6.1",
    short_description = "Enforce canonical Tailwind CSS classes.",
);

impl Rule for EnforceCanonicalClasses {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let config =
            serde_json::from_value::<DefaultRuleConfig<EnforceCanonicalClassesConfig>>(value)?
                .into_inner();
        let ignore = config
            .ignore
            .into_iter()
            .map(|pattern| Regex::new(&pattern).map_err(serde::de::Error::custom))
            .collect::<Result<_, _>>()?;
        Ok(Self(Box::new(EnforceCanonicalClassesOptions {
            collapse: config.collapse,
            logical: config.logical,
            root_font_size: config.root_font_size,
            ignore,
        })))
    }

    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        let mut seen = FxHashSet::default();
        let names = classes
            .iter()
            .map(|class| class.name)
            .filter(|name| !self.0.ignore.iter().any(|pattern| pattern.is_match(name)))
            .filter(|name| seen.insert(*name))
            .collect::<Vec<_>>();
        if names.is_empty() {
            return;
        }
        let Some(design) = ctx.tailwind_design_system() else { return };
        let canonical = native_canonical_suggestions(
            design,
            &names,
            oxc_tailwindcss::CanonicalizeOptions {
                rem: self.0.root_font_size,
                collapse: self.0.collapse,
                logical_to_physical: self.0.logical,
            },
        );
        for class in classes {
            let Some(suggestion) = canonical.get(class.name) else { continue };
            if suggestion.output == class.name {
                continue;
            }
            let (message, replacement) = if suggestion.input.len() > 1 {
                let message = format!(
                    "The classes: \"{}\" can be simplified to \"{}\".",
                    suggestion.input.join(", "),
                    suggestion.output
                );
                let replacement = if class.name == suggestion.input[0] {
                    suggestion.output.clone()
                } else {
                    String::new()
                };
                (message, replacement)
            } else {
                (
                    format!(
                        "The class: \"{}\" can be simplified to \"{}\".",
                        class.name, suggestion.output
                    ),
                    suggestion.output.clone(),
                )
            };
            let diagnostic = OxcDiagnostic::warn(message).with_label(class.span);
            ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(class.span, replacement));
        }
    }
}

fn native_canonical_suggestions(
    design: &oxc_tailwindcss::DesignSystem,
    classes: &[&str],
    options: oxc_tailwindcss::CanonicalizeOptions,
) -> FxHashMap<String, CanonicalClass> {
    let known = classes
        .iter()
        .copied()
        .filter(|class_name| design.is_known_class(class_name))
        .collect::<Vec<_>>();
    let canonical = design.canonicalize_classes(&known, options);
    let removed = known
        .iter()
        .copied()
        .filter(|class_name| !canonical.iter().any(|candidate| candidate == *class_name))
        .collect::<Vec<_>>();
    let mut result = FxHashMap::default();
    for class_name in classes {
        if canonical.iter().any(|candidate| candidate == *class_name)
            || !design.is_known_class(class_name)
        {
            result.insert(
                (*class_name).to_owned(),
                CanonicalClass {
                    input: vec![(*class_name).to_owned()],
                    output: (*class_name).to_owned(),
                },
            );
        }
    }
    for output in
        canonical.iter().filter(|output| !classes.iter().any(|class_name| output == class_name))
    {
        let necessary = removed
            .iter()
            .copied()
            .filter(|removed_class| {
                let subset = removed
                    .iter()
                    .copied()
                    .filter(|class_name| class_name != removed_class)
                    .collect::<Vec<_>>();
                !design.canonicalize_classes(&subset, options).contains(output)
            })
            .collect::<Vec<_>>();
        let input = necessary.iter().map(|class_name| (*class_name).to_owned()).collect::<Vec<_>>();
        for original in necessary {
            result.insert(
                original.to_owned(),
                CanonicalClass { input: input.clone(), output: output.to_string() },
            );
        }
    }
    result
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="flex [&:hover]:flex" />"#];
    let fail = vec![
        r#"<div className="[display:flex]" />"#,
        r#"<div className="[&:focus]:flex data-[selected]:flex" />"#,
        r#"<div className="[&:nth-child(2)]:flex [@media(pointer:fine)]:flex" />"#,
    ];
    let fix = vec![
        (r#"<div className="[display:flex]" />"#, r#"<div className="flex" />"#),
        (
            r#"<div className="[&:focus]:flex data-[selected]:flex" />"#,
            r#"<div className="focus:flex data-selected:flex" />"#,
        ),
        (
            r#"<div className="[&:nth-child(2)]:flex [@media(pointer:fine)]:flex" />"#,
            r#"<div className="nth-2:flex pointer-fine:flex" />"#,
        ),
    ];
    Tester::new(EnforceCanonicalClasses::NAME, EnforceCanonicalClasses::PLUGIN, pass, fail)
        .expect_fix(fix)
        .test_and_snapshot();
}
