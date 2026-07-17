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
pub struct NoUnknownClassesConfig {
    ignore: Vec<CompactStr>,
}

#[derive(Debug, Default, Clone)]
struct NoUnknownClassesOptions {
    ignore: Vec<Regex>,
}

#[derive(Debug, Default, Clone)]
pub struct NoUnknownClasses(Box<NoUnknownClassesOptions>);

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Disallows classes not recognized by the project's Tailwind design system.
    NoUnknownClasses,
    better_tailwindcss,
    correctness,
    config = NoUnknownClassesConfig,
    version = "4.6.1",
    short_description = "Disallow unknown Tailwind CSS classes.",
);

impl Rule for NoUnknownClasses {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let config = serde_json::from_value::<DefaultRuleConfig<NoUnknownClassesConfig>>(value)?
            .into_inner();
        let ignore = config
            .ignore
            .into_iter()
            .map(|pattern| Regex::new(&pattern).map_err(serde::de::Error::custom))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(Box::new(NoUnknownClassesOptions { ignore })))
    }

    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        if classes.is_empty() {
            return;
        }
        let names = classes.iter().map(|class| class.name).collect::<Vec<_>>();
        let Some(unknown): Option<Vec<String>> =
            ctx.tailwind_query("unknownClasses", &names, serde_json::json!({}))
        else {
            return;
        };
        for class in classes {
            if !unknown.iter().any(|unknown| unknown == class.name)
                || self.0.ignore.iter().any(|pattern| pattern.is_match(class.name))
                || is_marker_class(class.name)
            {
                continue;
            }
            ctx.diagnostic(
                OxcDiagnostic::warn(format!("Unknown class detected: {}", class.name))
                    .with_label(class.span),
            );
        }
    }
}

fn is_marker_class(class_name: &str) -> bool {
    let base = TailwindCandidate::parse(class_name).base;
    matches!(base, "group" | "peer") || base.starts_with("group/") || base.starts_with("peer/")
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="flex group peer/name" />"#];
    let fail = vec![r#"<div className="flex typo-class" />"#];
    Tester::new(NoUnknownClasses::NAME, NoUnknownClasses::PLUGIN, pass, fail)
        .with_tailwind_design_system(|request| {
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            let unknown = request["classes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|class| class.as_str() == Some("typo-class"))
                .cloned()
                .collect::<Vec<_>>();
            Ok(serde_json::to_string(&unknown).unwrap())
        })
        .test_and_snapshot();
}
