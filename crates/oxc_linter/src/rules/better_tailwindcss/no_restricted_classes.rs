use lazy_regex::Regex;
use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use oxc_str::CompactStr;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::{DefaultRuleConfig, Rule},
    utils::{tailwind_classes, tailwind_literal},
};

#[derive(Debug, Default, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct NoRestrictedClassesConfig {
    /// Classes or regular expression patterns that must not be used.
    restrict: Vec<RestrictionConfig>,
}

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(untagged)]
enum RestrictionConfig {
    Pattern(CompactStr),
    Detailed {
        /// A regular expression pattern matched against each class name.
        pattern: CompactStr,
        /// An optional replacement. Capture placeholders such as `$1` are supported.
        fix: Option<CompactStr>,
        /// An optional diagnostic message. Capture placeholders such as `$1` are supported.
        message: Option<CompactStr>,
    },
}

#[derive(Debug, Clone)]
struct Restriction {
    pattern: Regex,
    fix: Option<CompactStr>,
    message: Option<CompactStr>,
}

#[derive(Debug, Default, Clone)]
struct NoRestrictedClassesOptions {
    restrictions: Vec<Restriction>,
}

#[derive(Debug, Default, Clone)]
pub struct NoRestrictedClasses(Box<NoRestrictedClassesOptions>);

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Disallows configured Tailwind CSS classes or class-name patterns.
    ///
    /// ### Examples
    ///
    /// With `{ "restrict": ["^text-red-"] }`, this is **incorrect**:
    /// ```jsx
    /// <div className="text-red-500" />
    /// ```
    NoRestrictedClasses,
    better_tailwindcss,
    correctness,
    conditional_fix,
    config = NoRestrictedClassesConfig,
    version = "4.6.1",
    short_description = "Disallow restricted Tailwind CSS classes.",
);

impl Rule for NoRestrictedClasses {
    fn from_configuration(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let config = serde_json::from_value::<DefaultRuleConfig<NoRestrictedClassesConfig>>(value)?
            .into_inner();
        let restrictions = config
            .restrict
            .into_iter()
            .map(|restriction| {
                let (pattern, fix, message) = match restriction {
                    RestrictionConfig::Pattern(pattern) => (pattern, None, None),
                    RestrictionConfig::Detailed { pattern, fix, message } => {
                        (pattern, fix, message)
                    }
                };
                Regex::new(&pattern)
                    .map(|pattern| Restriction { pattern, fix, message })
                    .map_err(serde::de::Error::custom)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self(Box::new(NoRestrictedClassesOptions { restrictions })))
    }

    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        if self.0.restrictions.is_empty() {
            return;
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);

        for class in tailwind_classes(literal, content) {
            for restriction in &self.0.restrictions {
                let Some(captures) = restriction.pattern.captures(class.name) else { continue };
                let message = restriction.message.as_ref().map_or_else(
                    || format!(r#"Restricted class: "{}"."#, class.name),
                    |template| expand(template, &captures),
                );
                let diagnostic = OxcDiagnostic::warn(message).with_label(class.span);
                if let Some(fix) = &restriction.fix {
                    let replacement = expand(fix, &captures);
                    ctx.diagnostic_with_fix(diagnostic, |fixer| {
                        fixer.replace(class.span, replacement)
                    });
                } else {
                    ctx.diagnostic(diagnostic);
                }
                break;
            }
        }
    }
}

fn expand(template: &str, captures: &lazy_regex::Captures<'_>) -> String {
    let mut output = String::with_capacity(template.len());
    captures.expand(template, &mut output);
    output
}

#[test]
fn test() {
    use serde_json::json;

    use crate::tester::Tester;

    let config = Some(json!([{ "restrict": [
        "^container$",
        {
            "pattern": "^text-(red|blue)-(.*)$",
            "message": "Use a semantic color instead of $1.",
            "fix": "text-brand-$2"
        }
    ] }]));
    let pass = vec![
        (r#"<div className="text-brand-500 flex" />"#, config.clone()),
        (r#"const message = "container""#, config.clone()),
    ];
    let fail = vec![
        (r#"<div className="container flex" />"#, config.clone()),
        (r#"clsx("text-red-500")"#, config.clone()),
    ];
    let fix = vec![(r#"clsx("text-red-500")"#, r#"clsx("text-brand-500")"#, config)];

    Tester::new(NoRestrictedClasses::NAME, NoRestrictedClasses::PLUGIN, pass, fail)
        .expect_fix(fix)
        .test_and_snapshot();
}
