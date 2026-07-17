use std::cmp::Ordering;

use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use rustc_hash::FxHashMap;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::{tailwind_classes, tailwind_literal},
};

#[derive(Debug, Default, Clone, Copy, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ClassOrder {
    Asc,
    Desc,
    #[default]
    Official,
    Strict,
}

#[derive(Debug, Default, Clone, Copy, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
enum UnknownOrder {
    Asc,
    Desc,
    #[default]
    Preserve,
}

#[derive(Debug, Default, Clone, Copy, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
enum UnknownPosition {
    #[default]
    Start,
    End,
}

#[derive(Debug, Default, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EnforceConsistentClassOrder {
    order: ClassOrder,
    unknown_class_order: UnknownOrder,
    unknown_class_position: UnknownPosition,
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Sorts classes using the project's Tailwind `getClassOrder` implementation.
    EnforceConsistentClassOrder,
    better_tailwindcss,
    style,
    fix,
    config = EnforceConsistentClassOrder,
    version = "4.6.1",
    short_description = "Enforce Tailwind's canonical class order.",
);

impl Rule for EnforceConsistentClassOrder {
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
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        if classes.len() < 2 {
            return;
        }

        let first_sortable = usize::from(classes.first().is_some_and(|class| class.sticky));
        let last_sortable =
            classes.len() - usize::from(classes.last().is_some_and(|class| class.sticky));
        if last_sortable.saturating_sub(first_sortable) < 2 {
            return;
        }
        let mut sorted = classes[first_sortable..last_sortable]
            .iter()
            .map(|class| class.name)
            .collect::<Vec<_>>();

        match self.order {
            ClassOrder::Asc => sorted.sort_unstable(),
            ClassOrder::Desc => sorted.sort_by(|left, right| right.cmp(left)),
            ClassOrder::Official | ClassOrder::Strict => {
                let Some(order): Option<Vec<(String, Option<String>)>> =
                    ctx.tailwind_query("classOrder", &sorted, serde_json::json!({}))
                else {
                    return;
                };
                let order = order
                    .into_iter()
                    .map(|(class_name, order)| {
                        (class_name, order.and_then(|order| order.parse::<i128>().ok()))
                    })
                    .collect::<FxHashMap<_, _>>();
                sorted.sort_by(|left, right| compare_official(left, right, &order, self));
            }
        }

        if sorted
            .iter()
            .zip(&classes[first_sortable..last_sortable])
            .all(|(sorted, original)| *sorted == original.name)
        {
            return;
        }
        let mut replacement = String::with_capacity(content.len());
        let mut cursor = 0;
        for (index, class) in classes.iter().enumerate() {
            replacement.push_str(&content[cursor..class.start]);
            if (first_sortable..last_sortable).contains(&index) {
                replacement.push_str(sorted[index - first_sortable]);
            } else {
                replacement.push_str(class.name);
            }
            cursor = class.end;
        }
        replacement.push_str(&content[cursor..]);
        let diagnostic = OxcDiagnostic::warn(format!(
            "Incorrect class order. Expected \"{content}\" to be \"{replacement}\"."
        ))
        .with_label(literal.span);
        ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(literal.span, replacement));
    }
}

fn compare_official(
    left: &str,
    right: &str,
    orders: &FxHashMap<String, Option<i128>>,
    options: &EnforceConsistentClassOrder,
) -> Ordering {
    match (orders.get(left).copied().flatten(), orders.get(right).copied().flatten()) {
        (Some(left), Some(right)) => left.cmp(&right),
        (None, Some(_)) => match options.unknown_class_position {
            UnknownPosition::Start => Ordering::Less,
            UnknownPosition::End => Ordering::Greater,
        },
        (Some(_), None) => match options.unknown_class_position {
            UnknownPosition::Start => Ordering::Greater,
            UnknownPosition::End => Ordering::Less,
        },
        (None, None) => match options.unknown_class_order {
            UnknownOrder::Asc => left.cmp(right),
            UnknownOrder::Desc => right.cmp(left),
            UnknownOrder::Preserve => Ordering::Equal,
        },
    }
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="flex p-2 text-sm" />"#];
    let fail = vec![r#"<div className="text-sm flex p-2" />"#];
    let fix = vec![(
        r#"<div className="text-sm flex p-2" />"#,
        r#"<div className="flex p-2 text-sm" />"#,
    )];
    Tester::new(EnforceConsistentClassOrder::NAME, EnforceConsistentClassOrder::PLUGIN, pass, fail)
        .with_tailwind_design_system(|request| {
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            let result = request["classes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|class| {
                    let name = class.as_str().unwrap();
                    let order = match name {
                        "flex" => "1",
                        "p-2" => "2",
                        "text-sm" => "3",
                        _ => "4",
                    };
                    serde_json::json!([name, order])
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_string(&result).unwrap())
        })
        .expect_fix(fix)
        .test_and_snapshot();
}
