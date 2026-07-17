use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_span::Span;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::tailwind_literal,
};

fn unnecessary_whitespace_diagnostic(span: Span) -> OxcDiagnostic {
    OxcDiagnostic::warn("Unnecessary whitespace.").with_label(span)
}

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct NoUnnecessaryWhitespace {
    /// Preserve line breaks and indentation inside multiline class strings.
    allow_multiline: bool,
}

impl Default for NoUnnecessaryWhitespace {
    fn default() -> Self {
        Self { allow_multiline: true }
    }
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Disallows unnecessary whitespace in Tailwind CSS class strings.
    ///
    /// ### Why is this bad?
    ///
    /// Leading, trailing, or repeated whitespace makes class lists harder to scan and can make
    /// generated markup inconsistent.
    ///
    /// ### Examples
    ///
    /// Examples of **incorrect** code for this rule:
    /// ```jsx
    /// <div className="  rounded  underline  " />
    /// ```
    ///
    /// Examples of **correct** code for this rule:
    /// ```jsx
    /// <div className="rounded underline" />
    /// ```
    NoUnnecessaryWhitespace,
    better_tailwindcss,
    style,
    fix,
    config = NoUnnecessaryWhitespace,
    version = "4.6.1",
    short_description = "Disallow unnecessary whitespace in Tailwind CSS class strings.",
);

impl Rule for NoUnnecessaryWhitespace {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value::<DefaultRuleConfig<Self>>(value).map(DefaultRuleConfig::into_inner)
    }

    #[expect(clippy::cast_possible_truncation, reason = "Oxc source spans are limited to u32")]
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);

        let mut index = 0;
        while index < content.len() {
            if !content.as_bytes()[index].is_ascii_whitespace() {
                index += 1;
                continue;
            }
            let start = index;
            while index < content.len() && content.as_bytes()[index].is_ascii_whitespace() {
                index += 1;
            }
            let end = index;
            let whitespace = &content[start..end];
            let is_leading = start == 0;
            let is_trailing = end == content.len();

            let replacement = if (is_leading && !literal.keep_leading)
                || (is_trailing && !literal.keep_trailing)
            {
                Some("")
            } else if whitespace.contains('\n') && self.allow_multiline {
                let trimmed = whitespace.trim_start_matches(' ');
                (trimmed.len() != whitespace.len()).then_some(trimmed)
            } else if whitespace.len() > 1 {
                Some(" ")
            } else {
                None
            };

            if let Some(replacement) = replacement {
                let span =
                    Span::new(literal.span.start + start as u32, literal.span.start + end as u32);
                ctx.diagnostic_with_fix(unnecessary_whitespace_diagnostic(span), |fixer| {
                    fixer.replace(span, replacement.to_owned())
                });
            }
        }
    }
}

#[test]
fn test() {
    use serde_json::json;

    use crate::tester::Tester;

    let pass = vec![
        (r#"<div className="rounded underline" />"#, None),
        (r#"const message = "  not classes  ""#, None),
        ("const className = `rounded\n  underline`", None),
        (r#"clsx({ variant: "  not classes  " })"#, None),
    ];
    let fail = vec![
        (r#"<div className="  rounded  underline  " />"#, None),
        (r#"cn("flex   items-center")"#, None),
        ("const classes = `  rounded  `", None),
        ("const classes = `rounded  \n  underline`", None),
        ("const classes = `rounded\n  underline`", Some(json!([{ "allowMultiline": false }]))),
    ];
    let fix = vec![
        (
            r#"<div className="  rounded  underline  " />"#,
            r#"<div className="rounded underline" />"#,
            None,
        ),
        (r#"cn("flex   items-center")"#, r#"cn("flex items-center")"#, None),
        ("const classes = `  rounded  `", "const classes = `rounded`", None),
        (
            "const classes = `rounded\n  underline`",
            "const classes = `rounded underline`",
            Some(json!([{ "allowMultiline": false }])),
        ),
    ];

    Tester::new(NoUnnecessaryWhitespace::NAME, NoUnnecessaryWhitespace::PLUGIN, pass, fail)
        .expect_fix(fix)
        .test_and_snapshot();
}
