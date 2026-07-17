use lazy_regex::Regex;
use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_str::CompactStr;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::{TailwindCandidate, tailwind_classes, tailwind_literal},
};

#[derive(Debug, Default, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EnforceLogicalPropertiesConfig {
    ignore: Vec<CompactStr>,
}

#[derive(Debug, Default, Clone)]
struct EnforceLogicalPropertiesOptions {
    ignore: Vec<Regex>,
}

#[derive(Debug, Default, Clone)]
pub struct EnforceLogicalProperties(Box<EnforceLogicalPropertiesOptions>);

const LOGICAL_PREFIXES: &[(&str, &str)] = &[
    ("scroll-ml-", "scroll-ms-"),
    ("scroll-mr-", "scroll-me-"),
    ("scroll-pl-", "scroll-ps-"),
    ("scroll-pr-", "scroll-pe-"),
    ("scroll-mt-", "scroll-mbs-"),
    ("scroll-mb-", "scroll-mbe-"),
    ("scroll-pt-", "scroll-pbs-"),
    ("scroll-pb-", "scroll-pbe-"),
    ("border-l-", "border-s-"),
    ("border-r-", "border-e-"),
    ("border-t-", "border-bs-"),
    ("border-b-", "border-be-"),
    ("rounded-tl-", "rounded-ss-"),
    ("rounded-tr-", "rounded-se-"),
    ("rounded-br-", "rounded-ee-"),
    ("rounded-bl-", "rounded-es-"),
    ("rounded-l-", "rounded-s-"),
    ("rounded-r-", "rounded-e-"),
    ("min-h-", "min-block-"),
    ("min-w-", "min-inline-"),
    ("max-h-", "max-block-"),
    ("max-w-", "max-inline-"),
    ("pl-", "ps-"),
    ("pr-", "pe-"),
    ("pt-", "pbs-"),
    ("pb-", "pbe-"),
    ("ml-", "ms-"),
    ("mr-", "me-"),
    ("mt-", "mbs-"),
    ("mb-", "mbe-"),
    ("left-", "inset-s-"),
    ("right-", "inset-e-"),
    ("top-", "inset-bs-"),
    ("bottom-", "inset-be-"),
    ("h-", "block-"),
    ("w-", "inline-"),
];

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Replaces physical-direction utilities with valid logical-property equivalents.
    EnforceLogicalProperties,
    better_tailwindcss,
    style,
    fix,
    config = EnforceLogicalPropertiesConfig,
    version = "4.6.1",
    short_description = "Enforce logical-property Tailwind classes.",
);

impl Rule for EnforceLogicalProperties {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let config =
            serde_json::from_value::<DefaultRuleConfig<EnforceLogicalPropertiesConfig>>(value)?
                .into_inner();
        let ignore = config
            .ignore
            .into_iter()
            .map(|pattern| Regex::new(&pattern).map_err(serde::de::Error::custom))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(Box::new(EnforceLogicalPropertiesOptions { ignore })))
    }

    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        let mut candidates = Vec::new();
        for class in &classes {
            let parsed = TailwindCandidate::parse(class.name);
            if let Some(bases) = logical_bases(parsed.base) {
                candidates.extend(bases.into_iter().map(|base| parsed.replace_base(&base)));
            }
        }
        let Some(design) = ctx.tailwind_design_system() else { return };
        let unknown = design.unknown_classes(candidates.iter().map(String::as_str));

        for class in classes {
            if self.0.ignore.iter().any(|pattern| pattern.is_match(class.name)) {
                continue;
            }
            let parsed = TailwindCandidate::parse(class.name);
            let Some(bases) = logical_bases(parsed.base) else { continue };
            let replacements =
                bases.into_iter().map(|base| parsed.replace_base(&base)).collect::<Vec<_>>();
            if replacements.iter().any(|replacement| unknown.contains(&replacement.as_str())) {
                continue;
            }
            let replacement = replacements.join(" ");
            let noun = if replacements.len() == 1 { "class" } else { "classes" };
            let diagnostic = OxcDiagnostic::warn(format!(
                "Physical class detected. Replace \"{}\" with logical {noun} \"{}\".",
                class.name, replacement
            ))
            .with_label(class.span);
            ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(class.span, replacement));
        }
    }
}

fn logical_bases(base: &str) -> Option<Vec<String>> {
    let exact = match base {
        "border-l" => Some("border-s"),
        "border-r" => Some("border-e"),
        "border-t" => Some("border-bs"),
        "border-b" => Some("border-be"),
        "rounded-l" => Some("rounded-s"),
        "rounded-r" => Some("rounded-e"),
        "rounded-tl" => Some("rounded-ss"),
        "rounded-tr" => Some("rounded-se"),
        "rounded-br" => Some("rounded-ee"),
        "rounded-bl" => Some("rounded-es"),
        "text-left" => Some("text-start"),
        "text-right" => Some("text-end"),
        "float-left" => Some("float-start"),
        "float-right" => Some("float-end"),
        "clear-left" => Some("clear-start"),
        "clear-right" => Some("clear-end"),
        _ => None,
    };
    if let Some(exact) = exact {
        return Some(vec![exact.to_owned()]);
    }
    for &(from, to) in LOGICAL_PREFIXES {
        if let Some(value) = base.strip_prefix(from) {
            return Some(vec![format!("{to}{value}")]);
        }
    }
    base.strip_prefix("size-")
        .map(|value| vec![format!("block-{value}"), format!("inline-{value}")])
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="ms-2 text-start" />"#];
    let fail = vec![r#"<div className="ml-2 size-4" />"#];
    let fix = vec![(
        r#"<div className="ml-2 size-4" />"#,
        r#"<div className="ms-2 block-4 inline-4" />"#,
    )];
    Tester::new(EnforceLogicalProperties::NAME, EnforceLogicalProperties::PLUGIN, pass, fail)
        .expect_fix(fix)
        .test_and_snapshot();
}
