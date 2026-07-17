use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_span::Span;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::{tailwind_classes, tailwind_literal, tailwind_variants},
};

#[derive(Debug, Default, Clone, Copy, JsonSchema, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum GroupSeparator {
    EmptyLine,
    #[default]
    NewLine,
    Never,
}

#[derive(Debug, Default, Clone, Copy, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LineBreakStyle {
    #[default]
    Unix,
    Windows,
}

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(untagged)]
enum Indent {
    Spaces(u32),
    Name(IndentName),
}

impl Default for Indent {
    fn default() -> Self {
        Self::Spaces(2)
    }
}

#[derive(Debug, Clone, Copy, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
enum IndentName {
    Tab,
}

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EnforceConsistentLineWrappingConfig {
    print_width: usize,
    classes_per_line: usize,
    group: GroupSeparator,
    prefer_single_line: bool,
    indent: Indent,
    tab_width: usize,
    line_break_style: LineBreakStyle,
}

impl Default for EnforceConsistentLineWrappingConfig {
    fn default() -> Self {
        Self {
            print_width: 80,
            classes_per_line: 0,
            group: GroupSeparator::NewLine,
            prefer_single_line: false,
            indent: Indent::default(),
            tab_width: 1,
            line_break_style: LineBreakStyle::Unix,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct EnforceConsistentLineWrapping(Box<EnforceConsistentLineWrappingConfig>);

impl std::ops::Deref for EnforceConsistentLineWrapping {
    type Target = EnforceConsistentLineWrappingConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Wraps Tailwind class strings by width, class count, and variant group.
    EnforceConsistentLineWrapping,
    better_tailwindcss,
    style,
    fix,
    config = EnforceConsistentLineWrappingConfig,
    version = "4.6.1",
    short_description = "Enforce consistent line wrapping for Tailwind classes.",
);

impl Rule for EnforceConsistentLineWrapping {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value::<DefaultRuleConfig<EnforceConsistentLineWrappingConfig>>(value)
            .map(DefaultRuleConfig::into_inner)
            .map(Box::new)
            .map(Self)
    }

    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        if literal.keep_leading || literal.keep_trailing {
            return;
        }
        let content = ctx.source_range(literal.span);
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        if classes.is_empty() {
            return;
        }
        let names = classes.iter().map(|class| class.name).collect::<Vec<_>>();
        let single_line = names.join(" ");
        let groups = names
            .iter()
            .map(|class| {
                let (variants, _) = tailwind_variants(class);
                variants.join(":")
            })
            .collect::<Vec<_>>();
        let source = ctx.source_text();
        let line_start =
            source[..literal.span.start as usize].rfind('\n').map_or(0, |index| index + 1);
        let start_column =
            visual_width(&source[line_start..literal.span.start as usize], self.tab_width);
        let within_width =
            self.print_width == 0 || start_column + single_line.len() <= self.print_width;
        let within_count = self.classes_per_line == 0 || names.len() <= self.classes_per_line;
        let has_groups = self.group != GroupSeparator::Never
            && groups.windows(2).any(|groups| groups[0] != groups[1]);
        let use_single_line =
            within_width && within_count && (!has_groups || self.prefer_single_line);

        if use_single_line {
            if content == single_line {
                return;
            }
            report_fix(ctx, literal.span, content, single_line);
            return;
        }

        let line_break = match self.line_break_style {
            LineBreakStyle::Unix => "\n",
            LineBreakStyle::Windows => "\r\n",
        };
        let outer_indent = source[line_start..]
            .chars()
            .take_while(|character| matches!(character, ' ' | '\t'))
            .collect::<String>();
        let extra_indent = match self.indent {
            Indent::Spaces(count) => " ".repeat(count as usize),
            Indent::Name(IndentName::Tab) => "\t".to_owned(),
        };
        let inner_indent = format!("{outer_indent}{extra_indent}");
        let lines = self.wrap_lines(&names, &groups, visual_width(&inner_indent, self.tab_width));
        let body = lines
            .iter()
            .map(
                |line| {
                    if line.is_empty() { String::new() } else { format!("{inner_indent}{line}") }
                },
            )
            .collect::<Vec<_>>()
            .join(line_break);
        let template = format!("`{line_break}{body}{line_break}{outer_indent}`");

        let (span, replacement) = match node.kind() {
            AstKind::StringLiteral(string) => {
                let is_jsx = ctx
                    .nodes()
                    .ancestors(node.id())
                    .any(|ancestor| matches!(ancestor.kind(), AstKind::JSXAttribute(_)));
                let replacement = if is_jsx { format!("{{{template}}}") } else { template };
                (string.span, replacement)
            }
            AstKind::TemplateElement(_) => {
                let Some(template_literal) =
                    ctx.nodes().ancestors(node.id()).find_map(|ancestor| {
                        ancestor
                            .kind()
                            .as_template_literal()
                            .filter(|template| template.expressions.is_empty())
                    })
                else {
                    return;
                };
                (template_literal.span, template)
            }
            _ => return,
        };
        let before = ctx.source_range(span);
        if before != replacement {
            report_fix(ctx, span, before, replacement);
        }
    }
}

impl EnforceConsistentLineWrapping {
    fn wrap_lines(&self, names: &[&str], groups: &[String], indent_width: usize) -> Vec<String> {
        let mut lines = vec![String::new()];
        let mut classes_on_line = 0;
        for (index, class) in names.iter().enumerate() {
            let group_changed = index > 0 && groups[index] != groups[index - 1];
            if group_changed && self.group != GroupSeparator::Never {
                if self.group == GroupSeparator::EmptyLine {
                    lines.push(String::new());
                }
                lines.push(String::new());
                classes_on_line = 0;
            }
            let line = lines.last().unwrap();
            let next_width =
                indent_width + line.len() + usize::from(!line.is_empty()) + class.len();
            let over_width = self.print_width > 0 && next_width > self.print_width;
            let over_count = self.classes_per_line > 0 && classes_on_line >= self.classes_per_line;
            if !line.is_empty() && (over_width || over_count) {
                lines.push(String::new());
                classes_on_line = 0;
            }
            let line = lines.last_mut().unwrap();
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(class);
            classes_on_line += 1;
        }
        lines
    }
}

fn visual_width(value: &str, tab_width: usize) -> usize {
    value.chars().map(|character| if character == '\t' { tab_width } else { 1 }).sum()
}

fn report_fix(ctx: &LintContext<'_>, span: Span, before: &str, replacement: String) {
    let kind = if before.contains('\n') { "Unnecessary" } else { "Incorrect" };
    let diagnostic = OxcDiagnostic::warn(format!(
        "{kind} line wrapping. Expected \"{before}\" to be \"{replacement}\"."
    ))
    .with_label(span);
    ctx.diagnostic_with_fix(diagnostic, |fixer| fixer.replace(span, replacement));
}

#[test]
fn test() {
    use crate::tester::Tester;

    let pass = vec!["<div className={`\n  block\n  hover:flex\n`} />"];
    let fail = vec![r#"<div className="block hover:flex" />"#];
    let fix = vec![(
        r#"<div className="block hover:flex" />"#,
        "<div className={`\n  block\n  hover:flex\n`} />",
    )];
    Tester::new(
        EnforceConsistentLineWrapping::NAME,
        EnforceConsistentLineWrapping::PLUGIN,
        pass,
        fail,
    )
    .expect_fix(fix)
    .test_and_snapshot();
}
