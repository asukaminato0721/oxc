use lazy_regex::Regex;
use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_str::CompactStr;
use rustc_hash::FxHashMap;
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
    ignore: Vec<CompactStr>,
}

impl Default for EnforceCanonicalClassesConfig {
    fn default() -> Self {
        Self { collapse: true, logical: true, ignore: Vec::new() }
    }
}

#[derive(Debug, Default, Clone)]
struct EnforceCanonicalClassesOptions {
    collapse: bool,
    logical: bool,
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
        let mut names = classes
            .iter()
            .map(|class| class.name)
            .filter(|name| !self.0.ignore.iter().any(|pattern| pattern.is_match(name)))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            return;
        }
        let options = serde_json::json!({
            "collapse": self.0.collapse,
            "logicalToPhysical": self.0.logical,
        });
        let Some(canonical): Option<FxHashMap<String, CanonicalClass>> =
            ctx.tailwind_query("canonicalClasses", &names, options)
        else {
            return;
        };
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

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="flex" />"#];
    let fail = vec![r#"<div className="[display:flex]" />"#];
    let fix = vec![(r#"<div className="[display:flex]" />"#, r#"<div className="flex" />"#)];
    Tester::new(EnforceCanonicalClasses::NAME, EnforceCanonicalClasses::PLUGIN, pass, fail)
        .with_tailwind_design_system(|request| {
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            let mut result = serde_json::Map::new();
            for class in request["classes"].as_array().unwrap() {
                let class = class.as_str().unwrap();
                let output = if class == "[display:flex]" { "flex" } else { class };
                result.insert(
                    class.to_owned(),
                    serde_json::json!({ "input": [class], "output": output }),
                );
            }
            Ok(serde_json::Value::Object(result).to_string())
        })
        .expect_fix(fix)
        .test_and_snapshot();
}
