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
enum VariableSyntax {
    #[default]
    Shorthand,
    Variable,
}

#[derive(Debug, Default, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EnforceConsistentVariableSyntax {
    syntax: VariableSyntax,
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Enforces Tailwind v4's CSS-variable shorthand or the explicit `var(...)` spelling.
    EnforceConsistentVariableSyntax,
    better_tailwindcss,
    style,
    fix,
    config = EnforceConsistentVariableSyntax,
    version = "4.6.1",
    short_description = "Enforce consistent CSS variable syntax in Tailwind classes.",
);

impl Rule for EnforceConsistentVariableSyntax {
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
        if !content.contains("--") {
            return;
        }

        for class in tailwind_classes(literal, content) {
            let candidate = TailwindCandidate::parse(class.name);
            // Arbitrary property definitions such as `[--foo:bar]` are not variable values.
            if candidate.base.contains(':') {
                continue;
            }
            let replacement_base = match self.syntax {
                VariableSyntax::Shorthand => to_shorthand(candidate.base),
                VariableSyntax::Variable => to_variable(candidate.base),
            };
            let Some(replacement_base) = replacement_base else { continue };
            let replacement = candidate.replace_base(&replacement_base);
            let diagnostic =
                OxcDiagnostic::warn(format!("Incorrect variable syntax: {}.", class.name))
                    .with_label(class.span);
            ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(class.span, replacement));
        }
    }
}

fn to_shorthand(base: &str) -> Option<String> {
    let (start, end) = balanced(base, b'[', b']')?;
    let inner = &base[start + 1..end];
    let shorthand = if let Some(variable) = single_variable(inner) {
        variable.trim_matches('_')
    } else if is_shorthand(inner) {
        inner
    } else {
        return None;
    };
    Some(replace_range(base, start, end + 1, &format!("({shorthand})")))
}

fn to_variable(base: &str) -> Option<String> {
    if let Some((start, end)) = balanced(base, b'[', b']') {
        let inner = &base[start + 1..end];
        if inner.starts_with("var(") {
            return None;
        }
        if is_shorthand(inner) {
            return Some(replace_range(base, start, end + 1, &format!("[var({inner})]")));
        }
    }
    let (start, end) = balanced(base, b'(', b')')?;
    let inner = &base[start + 1..end];
    is_shorthand(inner).then(|| replace_range(base, start, end + 1, &format!("[var({inner})]")))
}

fn is_shorthand(value: &str) -> bool {
    let value = value.trim_start_matches('_');
    if value.match_indices("--").nth(1).is_some() {
        return false;
    }
    let Some(after) = value.strip_prefix("--") else { return false };
    if let Some(paren) = after.find('(') {
        let function = &after[..paren];
        if !function.is_empty()
            && function
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return false;
        }
    }
    true
}

fn single_variable(value: &str) -> Option<&str> {
    let inner = value.strip_prefix("var(")?.strip_suffix(')')?;
    let (_, end) = balanced(value, b'(', b')')?;
    (end + 1 == value.len() && is_shorthand(inner)).then_some(inner)
}

fn balanced(value: &str, open: u8, close: u8) -> Option<(usize, usize)> {
    let mut depth = 0_u32;
    let mut start = None;
    for (index, byte) in value.bytes().enumerate() {
        if byte == open {
            if depth == 0 {
                start = Some(index);
            }
            depth += 1;
        } else if byte == close && depth > 0 {
            depth -= 1;
            if depth == 0 {
                return Some((start?, index));
            }
        }
    }
    None
}

fn replace_range(value: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut output = String::with_capacity(value.len() - (end - start) + replacement.len());
    output.push_str(&value[..start]);
    output.push_str(replacement);
    output.push_str(&value[end..]);
    output
}

#[test]
fn test() {
    use serde_json::json;

    use crate::tester::Tester;

    let variable = Some(json!([{ "syntax": "variable" }]));
    let pass = vec![
        (r#"<div className="fill-(--brand)" />"#, None),
        (r#"<div className="fill-[var(--brand)]" />"#, variable.clone()),
        (r#"<div className="[--brand:red]" />"#, None),
        (r#"<div className="fill-[var(--one)_var(--two)]" />"#, None),
    ];
    let fail = vec![
        (r#"<div className="fill-[var(--brand)]" />"#, None),
        (r#"cn("fill-(--brand)")"#, variable.clone()),
    ];
    let fix = vec![
        (
            r#"<div className="fill-[var(--brand)]" />"#,
            r#"<div className="fill-(--brand)" />"#,
            None,
        ),
        (r#"cn("fill-(--brand)")"#, r#"cn("fill-[var(--brand)]")"#, variable),
    ];

    Tester::new(
        EnforceConsistentVariableSyntax::NAME,
        EnforceConsistentVariableSyntax::PLUGIN,
        pass,
        fail,
    )
    .expect_fix(fix)
    .test_and_snapshot();
}
