use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use rustc_hash::FxHashMap;

use crate::{
    AstNode, LintContext,
    rule::Rule,
    utils::{tailwind_classes, tailwind_literal, tailwind_variants},
};

const GLOBAL_ORDER: u32 = 1 << 30;

#[derive(Debug, Default, Clone)]
pub struct EnforceConsistentVariantOrder;

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Enforces the variant ordering reported by Tailwind v4's design system.
    EnforceConsistentVariantOrder,
    better_tailwindcss,
    style,
    fix,
    version = "4.6.1",
    short_description = "Enforce a consistent Tailwind variant order.",
);

impl Rule for EnforceConsistentVariantOrder {
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        if !classes.iter().any(|class| class.name.matches(':').count() > 1) {
            return;
        }
        let names = classes.iter().map(|class| class.name).collect::<Vec<_>>();
        let Some(orders): Option<FxHashMap<String, u32>> =
            ctx.tailwind_query("variantOrder", &names, serde_json::json!({}))
        else {
            return;
        };

        for class in classes {
            let (mut variants, utility) = tailwind_variants(class.name);
            if variants.len() < 2 {
                continue;
            }
            let prefix =
                variants.first().filter(|variant| !orders.contains_key(**variant)).copied();
            if prefix.is_some() {
                variants.remove(0);
            }
            if variants.len() < 2 || variants.iter().any(|variant| !orders.contains_key(*variant)) {
                continue;
            }
            let original = variants.clone();
            variants.sort_by(|left, right| {
                let left = orders[*left];
                let right = orders[*right];
                if left == right || (left < GLOBAL_ORDER && right < GLOBAL_ORDER) {
                    std::cmp::Ordering::Equal
                } else {
                    right.cmp(&left)
                }
            });
            if variants == original {
                continue;
            }
            let mut replacement = String::with_capacity(class.name.len());
            if let Some(prefix) = prefix {
                replacement.push_str(prefix);
                replacement.push(':');
            }
            for variant in variants {
                replacement.push_str(variant);
                replacement.push(':');
            }
            replacement.push_str(utility);
            let diagnostic = OxcDiagnostic::warn(format!(
                "Incorrect variant order. '{}' should be '{}'.",
                class.name, replacement
            ))
            .with_label(class.span);
            ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(class.span, replacement));
        }
    }
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="dark:hover:block" />"#];
    let fail = vec![r#"<div className="hover:dark:block" />"#];
    let fix = vec![(
        r#"<div className="hover:dark:block" />"#,
        r#"<div className="dark:hover:block" />"#,
    )];
    Tester::new(
        EnforceConsistentVariantOrder::NAME,
        EnforceConsistentVariantOrder::PLUGIN,
        pass,
        fail,
    )
    .with_tailwind_design_system(|_| {
        Ok(serde_json::json!({ "dark": 1073741825_u32, "hover": 2 }).to_string())
    })
    .expect_fix(fix)
    .test_and_snapshot();
}
