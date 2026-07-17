use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::{TailwindCandidate, tailwind_classes, tailwind_literal},
};

#[derive(Debug, Default, Clone, Copy, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ImportantPosition {
    Legacy,
    #[default]
    Recommended,
}

#[derive(Debug, Default, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EnforceConsistentImportantPosition {
    /// `legacy` places `!` before the utility; `recommended` uses Tailwind v4's trailing `!`.
    position: ImportantPosition,
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Enforces one position for Tailwind's important modifier.
    EnforceConsistentImportantPosition,
    better_tailwindcss,
    style,
    fix,
    config = EnforceConsistentImportantPosition,
    version = "4.6.1",
    short_description = "Enforce a consistent important modifier position.",
);

impl Rule for EnforceConsistentImportantPosition {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value::<DefaultRuleConfig<Self>>(value).map(DefaultRuleConfig::into_inner)
    }

    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        if !content.contains('!') {
            return;
        }

        for class in tailwind_classes(literal, content) {
            let candidate = TailwindCandidate::parse(class.name);
            let correct = match self.position {
                ImportantPosition::Legacy => candidate.important_at_start,
                ImportantPosition::Recommended => candidate.important_at_end,
            };
            if correct || (!candidate.important_at_start && !candidate.important_at_end) {
                continue;
            }
            let to_end = matches!(self.position, ImportantPosition::Recommended);
            let replacement = candidate.move_important(class.name, to_end);
            let diagnostic = OxcDiagnostic::warn(format!(
                "Incorrect important position. '{}' should be '{}'.",
                class.name, replacement
            ))
            .with_label(class.span);
            ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(class.span, replacement));
        }
    }
}

#[test]
fn test() {
    use serde_json::json;

    use crate::tester::Tester;

    let pass = vec![
        (r#"<div className="hover:bg-red-500!" />"#, None),
        (r#"<div className="hover:!bg-red-500" />"#, Some(json!([{ "position": "legacy" }]))),
    ];
    let fail = vec![
        (r#"<div className="hover:!bg-red-500" />"#, None),
        (r#"cn("-mt-2!")"#, Some(json!([{ "position": "legacy" }]))),
    ];
    let fix = vec![
        (
            r#"<div className="hover:!bg-red-500" />"#,
            r#"<div className="hover:bg-red-500!" />"#,
            None,
        ),
        (r#"cn("-mt-2!")"#, r#"cn("!-mt-2")"#, Some(json!([{ "position": "legacy" }]))),
    ];

    Tester::new(
        EnforceConsistentImportantPosition::NAME,
        EnforceConsistentImportantPosition::PLUGIN,
        pass,
        fail,
    )
    .expect_fix(fix)
    .test_and_snapshot();
}
