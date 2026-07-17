use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_span::Span;

use crate::{
    AstNode, LintContext,
    rule::Rule,
    utils::{TailwindCandidate, TailwindClass, tailwind_classes, tailwind_literal},
};

#[derive(Debug, Default, Clone)]
pub struct EnforceShorthandClasses;

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Collapses equivalent longhand utility groups into Tailwind shorthand utilities.
    EnforceShorthandClasses,
    better_tailwindcss,
    style,
    fix,
    version = "4.6.1",
    short_description = "Enforce shorthand Tailwind CSS classes.",
);

struct Group {
    indices: Vec<usize>,
    replacements: Vec<String>,
}

impl Rule for EnforceShorthandClasses {
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
        let mut groups = find_groups(&classes);
        let replacement_names = groups
            .iter()
            .flat_map(|group| group.replacements.iter().map(String::as_str))
            .collect::<Vec<_>>();
        let unknown: Vec<String> = ctx
            .tailwind_query("unknownClasses", &replacement_names, serde_json::json!({}))
            .unwrap_or(replacement_names.iter().map(|name| (*name).to_owned()).collect());
        groups.retain(|group| {
            !group.replacements.iter().any(|replacement| unknown.contains(replacement))
        });

        for group in groups {
            let mut indices = group.indices;
            indices.sort_unstable();
            let first = indices[0];
            let longhands =
                indices.iter().map(|&index| classes[index].name).collect::<Vec<_>>().join(" ");
            let shorthands = group.replacements.join(" ");
            let diagnostic = OxcDiagnostic::warn(format!(
                "Non-shorthand classes detected. Expected {longhands} to be {shorthands}."
            ))
            .with_label(classes[first].span);
            ctx.diagnostic_with_fix(diagnostic, |fixer| {
                let fixer = fixer.for_multifix();
                let mut fix = fixer.new_fix_with_capacity(indices.len());
                fix.push(fixer.replace(classes[first].span, shorthands));
                for &index in &indices[1..] {
                    let class = classes[index];
                    let mut start = class.start;
                    while start > 0 && content.as_bytes()[start - 1].is_ascii_whitespace() {
                        start -= 1;
                    }
                    let start = u32::try_from(start).expect("source offset should fit in u32");
                    fix.push(
                        fixer.delete_range(Span::new(literal.span.start + start, class.span.end)),
                    );
                }
                fix.with_message("Replace longhand utilities with their Tailwind shorthand")
            });
        }
    }
}

fn find_groups(classes: &[TailwindClass<'_>]) -> Vec<Group> {
    let mut groups = Vec::new();
    let mut used = vec![false; classes.len()];
    for &(patterns, replacements) in MAPPINGS {
        for first in 0..classes.len() {
            if used[first] {
                continue;
            }
            let first_candidate = TailwindCandidate::parse(classes[first].name);
            let Some(capture) = match_pattern(first_candidate.base, patterns[0]) else { continue };
            let shape = first_candidate.replace_base("");
            let mut indices = vec![first];
            for pattern in &patterns[1..] {
                let Some((index, _)) = classes.iter().enumerate().find(|(index, class)| {
                    if used[*index] || indices.contains(index) {
                        return false;
                    }
                    let candidate = TailwindCandidate::parse(class.name);
                    candidate.replace_base("") == shape
                        && match_pattern(candidate.base, pattern)
                            .is_some_and(|value| value == capture)
                }) else {
                    indices.clear();
                    break;
                };
                indices.push(index);
            }
            if indices.len() != patterns.len() {
                continue;
            }
            let replacements = replacements
                .iter()
                .map(|replacement| {
                    let base = replacement.strip_suffix('*').map_or_else(
                        || (*replacement).to_owned(),
                        |prefix| format!("{prefix}{capture}"),
                    );
                    first_candidate.replace_base(&base)
                })
                .collect();
            for &index in &indices {
                used[index] = true;
            }
            groups.push(Group { indices, replacements });
        }
    }
    groups
}

fn match_pattern<'a>(base: &'a str, pattern: &str) -> Option<&'a str> {
    pattern
        .strip_suffix('*')
        .map_or_else(|| (base == pattern).then_some(""), |prefix| base.strip_prefix(prefix))
}

type Mapping = (&'static [&'static str], &'static [&'static str]);
const MAPPINGS: &[Mapping] = &[
    (&["w-*", "h-*"], &["size-*"]),
    (&["ml-*", "mr-*", "mt-*", "mb-*"], &["m-*"]),
    (&["mx-*", "my-*"], &["m-*"]),
    (&["ms-*", "me-*"], &["mx-*"]),
    (&["ml-*", "mr-*"], &["mx-*"]),
    (&["mt-*", "mb-*"], &["my-*"]),
    (&["pl-*", "pr-*", "pt-*", "pb-*"], &["p-*"]),
    (&["px-*", "py-*"], &["p-*"]),
    (&["ps-*", "pe-*"], &["px-*"]),
    (&["pl-*", "pr-*"], &["px-*"]),
    (&["pt-*", "pb-*"], &["py-*"]),
    (&["border-t-*", "border-b-*", "border-l-*", "border-r-*"], &["border-*"]),
    (&["border-x-*", "border-y-*"], &["border-*"]),
    (&["border-s-*", "border-e-*"], &["border-x-*"]),
    (&["border-l-*", "border-r-*"], &["border-x-*"]),
    (&["border-t-*", "border-b-*"], &["border-y-*"]),
    (&["border-spacing-x-*", "border-spacing-y-*"], &["border-spacing-*"]),
    (&["rounded-tl-*", "rounded-tr-*", "rounded-bl-*", "rounded-br-*"], &["rounded-*"]),
    (&["rounded-t-*", "rounded-b-*"], &["rounded-*"]),
    (&["rounded-l-*", "rounded-r-*"], &["rounded-*"]),
    (&["rounded-tl-*", "rounded-tr-*"], &["rounded-t-*"]),
    (&["rounded-bl-*", "rounded-br-*"], &["rounded-b-*"]),
    (&["rounded-tl-*", "rounded-bl-*"], &["rounded-l-*"]),
    (&["rounded-tr-*", "rounded-br-*"], &["rounded-r-*"]),
    (&["scroll-mt-*", "scroll-mb-*", "scroll-ml-*", "scroll-mr-*"], &["scroll-m-*"]),
    (&["scroll-mx-*", "scroll-my-*"], &["scroll-m-*"]),
    (&["scroll-ms-*", "scroll-me-*"], &["scroll-mx-*"]),
    (&["scroll-ml-*", "scroll-mr-*"], &["scroll-mx-*"]),
    (&["scroll-mt-*", "scroll-mb-*"], &["scroll-my-*"]),
    (&["scroll-pt-*", "scroll-pb-*", "scroll-pl-*", "scroll-pr-*"], &["scroll-p-*"]),
    (&["scroll-px-*", "scroll-py-*"], &["scroll-p-*"]),
    (&["scroll-pl-*", "scroll-pr-*"], &["scroll-px-*"]),
    (&["scroll-ps-*", "scroll-pe-*"], &["scroll-px-*"]),
    (&["scroll-pt-*", "scroll-pb-*"], &["scroll-py-*"]),
    (&["top-*", "right-*", "bottom-*", "left-*"], &["inset-*"]),
    (&["inset-x-*", "inset-y-*"], &["inset-*"]),
    (&["divide-x-*", "divide-y-*"], &["divide-*"]),
    (&["space-x-*", "space-y-*"], &["space-*"]),
    (&["gap-x-*", "gap-y-*"], &["gap-*"]),
    (&["translate-x-*", "translate-y-*"], &["translate-*"]),
    (&["rotate-x-*", "rotate-y-*"], &["rotate-*"]),
    (&["skew-x-*", "skew-y-*"], &["skew-*"]),
    (&["scale-x-*", "scale-y-*", "scale-z-*"], &["scale-*", "scale-3d"]),
    (&["scale-x-*", "scale-y-*"], &["scale-*"]),
    (&["content-*", "justify-content-*"], &["place-content-*"]),
    (&["items-*", "justify-items-*"], &["place-items-*"]),
    (&["self-*", "justify-self-*"], &["place-self-*"]),
    (&["overflow-hidden", "text-ellipsis", "whitespace-nowrap"], &["truncate"]),
];

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec![r#"<div className="size-4 truncate" />"#];
    let fail = vec![r#"<div className="w-4 h-4" />"#];
    let fix = vec![(r#"<div className="w-4 h-4" />"#, r#"<div className="size-4" />"#)];
    Tester::new(EnforceShorthandClasses::NAME, EnforceShorthandClasses::PLUGIN, pass, fail)
        .with_tailwind_design_system(|_| Ok("[]".to_owned()))
        .expect_fix(fix)
        .test_and_snapshot();
}
