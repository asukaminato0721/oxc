use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_span::Span;
use smallvec::SmallVec;

use crate::{
    AstNode, LintContext,
    rule::Rule,
    utils::{tailwind_classes, tailwind_literal},
};

fn duplicate_class_diagnostic(class_name: &str, span: Span) -> OxcDiagnostic {
    OxcDiagnostic::warn(format!(r#"Duplicate classname: "{class_name}"."#)).with_label(span)
}

#[derive(Debug, Default, Clone)]
pub struct NoDuplicateClasses;

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Disallows duplicate classes in Tailwind CSS class strings.
    ///
    /// ### Why is this bad?
    ///
    /// Duplicate classes add noise and can conceal accidental class composition mistakes.
    ///
    /// ### Examples
    ///
    /// Examples of **incorrect** code for this rule:
    /// ```jsx
    /// <div className="rounded underline rounded" />
    /// ```
    ///
    /// Examples of **correct** code for this rule:
    /// ```jsx
    /// <div className="rounded underline" />
    /// ```
    NoDuplicateClasses,
    better_tailwindcss,
    style,
    fix,
    version = "4.6.1",
    short_description = "Disallow duplicate class names in Tailwind CSS class strings.",
);

impl Rule for NoDuplicateClasses {
    #[expect(clippy::cast_possible_truncation, reason = "Oxc source spans are limited to u32")]
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        if content.bytes().all(|byte| !byte.is_ascii_whitespace()) {
            return;
        }

        let classes: SmallVec<[_; 16]> = tailwind_classes(literal, content).collect();
        let mut seen: SmallVec<[&str; 16]> = SmallVec::new();
        let mut index = 0;
        while index < classes.len() {
            let class = classes[index];
            if seen.contains(&class.name) && !class.sticky {
                let mut removal_start = class.start;
                while removal_start > 0
                    && content.as_bytes()[removal_start - 1].is_ascii_whitespace()
                {
                    removal_start -= 1;
                }
                let mut removal_end = class.end;
                // Merge a run of the same duplicate into one fix. Adjacent fix ranges are treated
                // as conflicting by the fixer, so separate diagnostics would leave one duplicate
                // behind in `flex flex flex`.
                while index + 1 < classes.len() && classes[index + 1].name == class.name {
                    index += 1;
                    removal_end = classes[index].end;
                }
                let removal_span = Span::new(
                    literal.span.start + removal_start as u32,
                    literal.span.start + removal_end as u32,
                );
                ctx.diagnostic_with_fix(
                    duplicate_class_diagnostic(class.name, class.span),
                    |fixer| fixer.delete_range(removal_span),
                );
            } else {
                seen.push(class.name);
            }
            index += 1;
        }
    }
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![
        r#"<div className="rounded underline" />"#,
        r#"const label = "rounded rounded""#,
        r#"clsx("rounded", foo("underline underline"))"#,
        r#"clsx({ variant: "rounded rounded" })"#,
        r#"const classes = { "rounded rounded": condition }"#,
        r#"tw`rounded ${condition ? "underline" : "italic"} rounded`"#,
    ];
    let fail = vec![
        r#"<div className="rounded underline rounded" />"#,
        r#"<div class={'p-4  p-4 text-sm'} />"#,
        r#"clsx("flex flex")"#,
        r#"clsx({ "block block": condition })"#,
        r#"const classNames = `px-2 px-2`"#,
    ];
    let fix = vec![
        (
            r#"<div className="rounded underline rounded" />"#,
            r#"<div className="rounded underline" />"#,
        ),
        (r#"clsx("flex flex")"#, r#"clsx("flex")"#),
        (r#"clsx("flex flex flex")"#, r#"clsx("flex")"#),
        (r#"const classNames = `px-2 px-2`"#, r#"const classNames = `px-2`"#),
    ];

    Tester::new(NoDuplicateClasses::NAME, NoDuplicateClasses::PLUGIN, pass, fail)
        .expect_fix(fix)
        .test_and_snapshot();
}
