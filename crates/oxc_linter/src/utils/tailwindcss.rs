use oxc_ast::{
    AstKind,
    ast::{Expression, MemberExpression, PropertyKey},
};
use oxc_span::{GetSpan, Span};
use smallvec::SmallVec;

use crate::{AstNode, LintContext};

const DEFAULT_CALLEES: [&str; 14] = [
    "cc", "clb", "clsx", "cn", "cnb", "ctl", "cva", "cx", "dcnb", "objstr", "tv", "tw", "twJoin",
    "twMerge",
];
const DEFAULT_TAGS: [&str; 3] = ["tw", "twc", "twx"];

/// A source slice known to contain Tailwind CSS classes.
#[derive(Debug, Clone, Copy)]
pub struct TailwindLiteral {
    pub span: Span,
    /// A template element touching an interpolation must keep the class at this edge. Removing it
    /// could join a dynamic value and a static class into a different class name.
    pub keep_leading: bool,
    pub keep_trailing: bool,
}

/// A single class name and its source location inside a [`TailwindLiteral`].
#[derive(Debug, Clone, Copy)]
pub struct TailwindClass<'a> {
    pub name: &'a str,
    pub span: Span,
    pub start: usize,
    pub end: usize,
    pub sticky: bool,
}

/// Iterate the whitespace-separated classes in a Tailwind literal without allocating.
pub fn tailwind_classes(
    literal: TailwindLiteral,
    content: &str,
) -> impl Iterator<Item = TailwindClass<'_>> {
    let mut index = 0;
    std::iter::from_fn(move || {
        while index < content.len() && content.as_bytes()[index].is_ascii_whitespace() {
            index += 1;
        }
        let start = index;
        while index < content.len() && !content.as_bytes()[index].is_ascii_whitespace() {
            index += 1;
        }
        let end = index;
        let start_offset = u32::try_from(start).expect("source offset should fit in u32");
        let end_offset = u32::try_from(end).expect("source offset should fit in u32");
        (start != end).then(|| TailwindClass {
            name: &content[start..end],
            span: Span::new(literal.span.start + start_offset, literal.span.start + end_offset),
            start,
            end,
            sticky: (literal.keep_leading && start == 0)
                || (literal.keep_trailing && end == content.len()),
        })
    })
}

/// Syntactic parts of a Tailwind candidate. Colons inside arbitrary variants and values are
/// ignored, so `hover:[&:focus]:-mt-[var(--gap)]!` is split at the final top-level colon.
#[derive(Debug, Clone, Copy)]
pub struct TailwindCandidate<'a> {
    variant_prefix: &'a str,
    before_base: &'a str,
    pub base: &'a str,
    after_base: &'a str,
    pub important_at_start: bool,
    pub important_at_end: bool,
}

impl<'a> TailwindCandidate<'a> {
    pub fn parse(class_name: &'a str) -> Self {
        let bytes = class_name.as_bytes();
        let mut square_depth = 0_u32;
        let mut paren_depth = 0_u32;
        let mut escaped = false;
        let mut segment_start = 0;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'[' => square_depth += 1,
                b']' => square_depth = square_depth.saturating_sub(1),
                b'(' => paren_depth += 1,
                b')' => paren_depth = paren_depth.saturating_sub(1),
                b':' if square_depth == 0 && paren_depth == 0 => segment_start = index + 1,
                _ => {}
            }
        }

        let segment = &class_name[segment_start..];
        let important_at_end = segment.ends_with('!');
        let without_end = segment.strip_suffix('!').unwrap_or(segment);
        let mut base_start = 0;
        let mut important_at_start = false;
        // Accept both modifier orders. Tailwind's printer normalizes these when a rule changes the
        // important position, while base-only replacements preserve the user's spelling.
        for _ in 0..2 {
            match without_end.as_bytes().get(base_start) {
                Some(b'!') if !important_at_start => {
                    important_at_start = true;
                    base_start += 1;
                }
                Some(b'-') => base_start += 1,
                _ => break,
            }
        }
        let base_end = without_end.len();
        Self {
            variant_prefix: &class_name[..segment_start],
            before_base: &class_name[..segment_start + base_start],
            base: &class_name[segment_start + base_start..segment_start + base_end],
            after_base: &class_name[segment_start + base_end..],
            important_at_start,
            important_at_end,
        }
    }

    pub fn replace_base(self, base: &str) -> String {
        let mut output =
            String::with_capacity(self.before_base.len() + base.len() + self.after_base.len());
        output.push_str(self.before_base);
        output.push_str(base);
        output.push_str(self.after_base);
        output
    }

    pub fn move_important(self, class_name: &str, to_end: bool) -> String {
        let mut output = String::with_capacity(class_name.len());
        output.push_str(self.variant_prefix);
        if !to_end {
            output.push('!');
        }
        output.extend(
            self.before_base[self.variant_prefix.len()..]
                .chars()
                .filter(|&character| character != '!'),
        );
        output.push_str(self.base);
        if to_end {
            output.push('!');
        }
        output
    }
}

/// Split top-level variants from the utility segment without allocating variant strings.
pub fn tailwind_variants(class_name: &str) -> (SmallVec<[&str; 4]>, &str) {
    let bytes = class_name.as_bytes();
    let mut variants = SmallVec::new();
    let mut segment_start = 0;
    let mut square_depth = 0_u32;
    let mut paren_depth = 0_u32;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'[' => square_depth += 1,
            b']' => square_depth = square_depth.saturating_sub(1),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b':' if square_depth == 0 && paren_depth == 0 => {
                variants.push(&class_name[segment_start..index]);
                segment_start = index + 1;
            }
            _ => {}
        }
    }
    (variants, &class_name[segment_start..])
}

/// Return the editable content span when `node` is a string or template element in one of the
/// class-containing locations supported by `eslint-plugin-better-tailwindcss` by default.
pub fn tailwind_literal(node: &AstNode<'_>, ctx: &LintContext<'_>) -> Option<TailwindLiteral> {
    let (span, mut keep_leading, keep_trailing) = match node.kind() {
        AstKind::StringLiteral(literal) => {
            let source = ctx.source_range(literal.span);
            let bytes = source.as_bytes();
            if bytes.len() < 2
                || !matches!(bytes[0], b'\'' | b'"')
                || bytes[bytes.len() - 1] != bytes[0]
            {
                return None;
            }
            (Span::new(literal.span.start + 1, literal.span.end - 1), false, false)
        }
        AstKind::TemplateElement(element) => {
            // TemplateElement spans exclude the surrounding backtick / `${` / `}` delimiters.
            (element.span, true, !element.tail)
        }
        _ => return None,
    };

    let mut child_span = node.span();
    let mut is_object_key = false;
    for ancestor in ctx.nodes().ancestors(node.id()) {
        match ancestor.kind() {
            AstKind::TemplateLiteral(template) => {
                if template.quasis.first().is_some_and(|first| first.span == child_span) {
                    keep_leading = false;
                }
                child_span = template.span;
            }
            AstKind::ObjectProperty(property) => {
                // The default class helper selectors include object keys, but deliberately do not
                // treat arbitrary object values as classes.
                let is_key = match &property.key {
                    PropertyKey::StringLiteral(key) => key.span == child_span,
                    _ => false,
                };
                if !is_key {
                    return None;
                }
                is_object_key = true;
                child_span = property.span;
            }
            AstKind::JSXAttribute(attribute) => {
                if is_object_key {
                    return None;
                }
                return attribute.name.as_identifier().and_then(|name| {
                    matches!(name.name.as_str(), "class" | "className").then_some(TailwindLiteral {
                        span,
                        keep_leading,
                        keep_trailing,
                    })
                });
            }
            AstKind::CallExpression(call) => {
                return call.callee_name().and_then(|name| {
                    DEFAULT_CALLEES.contains(&name).then_some(TailwindLiteral {
                        span,
                        keep_leading,
                        keep_trailing,
                    })
                });
            }
            AstKind::TaggedTemplateExpression(tagged) => {
                if is_object_key {
                    return None;
                }
                return expression_name(&tagged.tag).and_then(|name| {
                    DEFAULT_TAGS.contains(&name).then_some(TailwindLiteral {
                        span,
                        keep_leading,
                        keep_trailing,
                    })
                });
            }
            AstKind::VariableDeclarator(declarator) => {
                if is_object_key {
                    return None;
                }
                return declarator.id.get_binding_identifier().and_then(|ident| {
                    matches!(
                        ident.name.as_str(),
                        "class" | "classes" | "className" | "classNames" | "style" | "styles"
                    )
                    .then_some(TailwindLiteral {
                        span,
                        keep_leading,
                        keep_trailing,
                    })
                });
            }
            kind if kind.is_function_like() => return None,
            AstKind::Program(_) => return None,
            _ => child_span = ancestor.span(),
        }
    }

    None
}

fn expression_name<'a>(expression: &'a Expression<'a>) -> Option<&'a str> {
    expression
        .get_identifier_reference()
        .map(|ident| ident.name.as_str())
        .or_else(|| expression.get_member_expr().and_then(MemberExpression::static_property_name))
}
