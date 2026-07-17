use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;

use crate::{
    AstNode, LintContext,
    rule::Rule,
    utils::{TailwindCandidate, tailwind_classes, tailwind_literal},
};

#[derive(Debug, Default, Clone)]
pub struct NoDeprecatedClasses;

enum DeprecatedClass {
    Replacement(String),
    Removed,
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Disallows utilities removed or renamed in Tailwind CSS v4.
    NoDeprecatedClasses,
    better_tailwindcss,
    style,
    conditional_fix,
    version = "4.6.1",
    short_description = "Disallow deprecated Tailwind CSS classes.",
);

impl Rule for NoDeprecatedClasses {
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);

        for class in tailwind_classes(literal, content) {
            let candidate = TailwindCandidate::parse(class.name);
            let Some(replacement_base) = replacement(candidate.base) else { continue };
            match replacement_base {
                DeprecatedClass::Replacement(base) => {
                    let replacement = candidate.replace_base(&base);
                    let diagnostic = OxcDiagnostic::warn(format!(
                        "Deprecated class detected. Replace \"{}\" with \"{}\".",
                        class.name, replacement
                    ))
                    .with_label(class.span);
                    ctx.diagnostic_with_fix(diagnostic, |fixer| {
                        fixer.replace(class.span, replacement)
                    });
                }
                DeprecatedClass::Removed => ctx.diagnostic(
                    OxcDiagnostic::warn(format!(
                        "Class \"{}\" is deprecated. Check the Tailwind CSS v4 upgrade guide.",
                        class.name
                    ))
                    .with_label(class.span),
                ),
            }
        }
    }
}

fn replacement(base: &str) -> Option<DeprecatedClass> {
    let exact = match base {
        "shadow" => Some("shadow-sm"),
        "inset-shadow" => Some("inset-shadow-sm"),
        "drop-shadow" => Some("drop-shadow-sm"),
        "blur" => Some("blur-sm"),
        "backdrop-blur" => Some("backdrop-blur-sm"),
        "rounded" => Some("rounded-sm"),
        "flex-shrink" => Some("shrink"),
        "flex-grow" => Some("grow"),
        "overflow-ellipsis" => Some("text-ellipsis"),
        "decoration-slice" => Some("box-decoration-slice"),
        "decoration-clone" => Some("box-decoration-clone"),
        "bg-left-top" => Some("bg-top-left"),
        "bg-left-bottom" => Some("bg-bottom-left"),
        "bg-right-top" => Some("bg-top-right"),
        "bg-right-bottom" => Some("bg-bottom-right"),
        "object-left-top" => Some("object-top-left"),
        "object-left-bottom" => Some("object-bottom-left"),
        "object-right-top" => Some("object-top-right"),
        "object-right-bottom" => Some("object-bottom-right"),
        _ => None,
    };
    if let Some(exact) = exact {
        return Some(DeprecatedClass::Replacement(exact.to_owned()));
    }
    for prefix in [
        "bg-opacity-",
        "text-opacity-",
        "border-opacity-",
        "divide-opacity-",
        "ring-opacity-",
        "placeholder-opacity-",
    ] {
        if base.starts_with(prefix) {
            return Some(DeprecatedClass::Removed);
        }
    }
    base.strip_prefix("flex-shrink-")
        .map(|value| DeprecatedClass::Replacement(format!("shrink-{value}")))
        .or_else(|| {
            base.strip_prefix("flex-grow-")
                .map(|value| DeprecatedClass::Replacement(format!("grow-{value}")))
        })
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="shadow-sm grow text-ellipsis" />"#];
    let fail = vec![r#"<div className="shadow hover:flex-shrink-0" />"#, r#"cn("bg-opacity-50")"#];
    let fix = vec![(
        r#"<div className="shadow hover:flex-shrink-0" />"#,
        r#"<div className="shadow-sm hover:shrink-0" />"#,
    )];

    Tester::new(NoDeprecatedClasses::NAME, NoDeprecatedClasses::PLUGIN, pass, fail)
        .expect_fix(fix)
        .test_and_snapshot();
}
