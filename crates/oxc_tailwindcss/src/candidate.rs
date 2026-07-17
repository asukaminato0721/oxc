use compact_str::CompactString;

use crate::{
    DesignSystem, VariantRegistration,
    segment::{decode_arbitrary, is_valid_arbitrary, segment},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modifier {
    Arbitrary(CompactString),
    Named(CompactString),
}

impl Modifier {
    pub fn value(&self) -> &str {
        match self {
            Self::Arbitrary(value) | Self::Named(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbitraryValue {
    pub data_type: Option<CompactString>,
    pub value: CompactString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedValue {
    pub value: CompactString,
    pub fraction: Option<CompactString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateValue {
    Arbitrary(ArbitraryValue),
    Named(NamedValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateKind {
    Arbitrary { property: CompactString, value: CompactString, modifier: Option<Modifier> },
    Static { root: CompactString },
    Functional { root: CompactString, value: Option<CandidateValue>, modifier: Option<Modifier> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    pub variants: Vec<Variant>,
    pub important: bool,
    pub raw: CompactString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantValue {
    Arbitrary(CompactString),
    Named(CompactString),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantKind {
    Arbitrary { selector: CompactString, relative: bool },
    Static { root: CompactString },
    Functional { root: CompactString, value: Option<VariantValue>, modifier: Option<Modifier> },
    Compound { root: CompactString, variant: Box<Variant>, modifier: Option<Modifier> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub kind: VariantKind,
}

impl Candidate {
    pub fn parse(input: &str, design: &DesignSystem) -> Vec<Self> {
        let mut raw_variants = segment(input, b':');
        if let Some(prefix) = design.prefix() {
            if raw_variants.len() == 1 || raw_variants.first().copied() != Some(prefix) {
                return Vec::new();
            }
            raw_variants.remove(0);
        }

        let Some(mut base) = raw_variants.pop() else { return Vec::new() };
        let mut variants = Vec::with_capacity(raw_variants.len());
        for raw_variant in raw_variants.into_iter().rev() {
            let Some(variant) = Variant::parse(raw_variant, design) else {
                return Vec::new();
            };
            variants.push(variant);
        }

        let important = base.ends_with('!') || base.starts_with('!');
        if base.ends_with('!') {
            base = &base[..base.len() - 1];
        } else if base.starts_with('!') {
            base = &base[1..];
        }

        let mut candidates = Vec::new();
        if design.has_static_utility(base) && !base.contains(['[', '/', '(']) {
            candidates.push(Self {
                kind: CandidateKind::Static { root: CompactString::new(base) },
                variants: variants.clone(),
                important,
                raw: CompactString::new(input),
            });
        }

        let modifier_parts = segment(base, b'/');
        if modifier_parts.len() > 2 {
            return Vec::new();
        }
        let base_without_modifier = modifier_parts[0];
        let modifier_segment = modifier_parts.get(1).copied();
        let modifier = match modifier_segment {
            Some(value) => {
                let Some(modifier) = parse_modifier(value) else { return Vec::new() };
                Some(modifier)
            }
            None => None,
        };

        if let Some(body) =
            base_without_modifier.strip_prefix('[').and_then(|value| value.strip_suffix(']'))
        {
            let Some(first) = body.as_bytes().first().copied() else { return Vec::new() };
            if first != b'-' && !first.is_ascii_lowercase() {
                return Vec::new();
            }
            let Some(colon) = body.find(':') else { return Vec::new() };
            if colon == 0 || colon + 1 == body.len() {
                return Vec::new();
            }
            let value = decode_arbitrary(&body[colon + 1..]);
            if !is_valid_arbitrary(&value) {
                return Vec::new();
            }
            candidates.push(Self {
                kind: CandidateKind::Arbitrary {
                    property: CompactString::new(&body[..colon]),
                    value: CompactString::new(value),
                    modifier,
                },
                variants,
                important,
                raw: CompactString::new(input),
            });
            return candidates;
        }

        let roots = find_utility_roots(base_without_modifier, design);
        for (root, raw_value) in roots {
            let value = match raw_value {
                None => None,
                Some(raw_value) => {
                    let Some(value) =
                        parse_candidate_value(raw_value, modifier_segment, modifier.as_ref())
                    else {
                        continue;
                    };
                    Some(value)
                }
            };
            candidates.push(Self {
                kind: CandidateKind::Functional {
                    root: CompactString::new(root),
                    value,
                    modifier: modifier.clone(),
                },
                variants: variants.clone(),
                important,
                raw: CompactString::new(input),
            });
        }
        candidates
    }

    pub fn print(&self, design: &DesignSystem) -> String {
        let mut parts = self.variants.iter().rev().map(Variant::print).collect::<Vec<_>>();
        if let Some(prefix) = design.prefix() {
            parts.insert(0, prefix.to_owned());
        }
        let mut base = match &self.kind {
            CandidateKind::Static { root } => root.to_string(),
            CandidateKind::Functional { root, value, modifier } => {
                let mut base = root.to_string();
                if let Some(value) = value {
                    base.push('-');
                    match value {
                        CandidateValue::Named(value) => base.push_str(&value.value),
                        CandidateValue::Arbitrary(value) => {
                            if let Some(variable) = css_variable(&value.value) {
                                base.push('(');
                                if let Some(data_type) = &value.data_type {
                                    base.push_str(data_type);
                                    base.push(':');
                                }
                                base.push_str(&print_arbitrary(variable));
                                base.push(')');
                            } else {
                                base.push('[');
                                if let Some(data_type) = &value.data_type {
                                    base.push_str(data_type);
                                    base.push(':');
                                }
                                base.push_str(&print_arbitrary(&value.value));
                                base.push(']');
                            }
                        }
                    }
                }
                base.push_str(&print_modifier(modifier.as_ref()));
                base
            }
            CandidateKind::Arbitrary { property, value, modifier } => {
                let mut base = format!("[{property}:{}]", print_arbitrary(value));
                base.push_str(&print_modifier(modifier.as_ref()));
                base
            }
        };
        if self.important {
            base.push('!');
        }
        parts.push(base);
        parts.join(":")
    }
}

impl Variant {
    pub fn parse(input: &str, design: &DesignSystem) -> Option<Self> {
        if let Some(body) = input.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
            if body.starts_with('@') && body.contains('&') {
                return None;
            }
            let mut selector = decode_arbitrary(body);
            if selector.trim().is_empty() || !is_valid_arbitrary(&selector) {
                return None;
            }
            let relative = selector.starts_with(['>', '+', '~']);
            if !relative && !selector.starts_with('@') && !selector.contains('&') {
                selector = format!("&:is({selector})");
            }
            return Some(Self {
                kind: VariantKind::Arbitrary { selector: CompactString::new(selector), relative },
            });
        }

        let parts = segment(input, b'/');
        if parts.len() > 2 {
            return None;
        }
        let variant_without_modifier = parts[0];
        let mut modifier_segment = parts.get(1).copied();
        for (root, mut value) in find_variant_roots(variant_without_modifier, design) {
            match design.variant_registration(root)? {
                VariantRegistration::Static { .. } => {
                    if value.is_some() || modifier_segment.is_some() {
                        return None;
                    }
                    return Some(Self {
                        kind: VariantKind::Static { root: CompactString::new(root) },
                    });
                }
                VariantRegistration::Functional { .. } => {
                    let modifier = match modifier_segment {
                        Some(raw) => Some(parse_modifier(raw)?),
                        None => None,
                    };
                    let value = match value {
                        None => None,
                        Some(raw) if raw.ends_with(']') && !raw.starts_with('[') => continue,
                        Some(raw) if raw.ends_with(')') && !raw.starts_with('(') => continue,
                        Some(raw) if raw.starts_with(['[', '(']) => Some(parse_variant_value(raw)?),
                        Some(raw) => {
                            let Some(value) = parse_variant_value(raw) else { continue };
                            Some(value)
                        }
                    };
                    return Some(Self {
                        kind: VariantKind::Functional {
                            root: CompactString::new(root),
                            value,
                            modifier,
                        },
                    });
                }
                VariantRegistration::Compound { .. } => {
                    let Some(mut raw_subvariant) = value.take() else { return None };
                    let forwarded;
                    if modifier_segment.is_some() && matches!(root, "not" | "has" | "in") {
                        forwarded = format!("{raw_subvariant}/{}", modifier_segment.take()?);
                        raw_subvariant = &forwarded;
                    }
                    let variant = Self::parse(raw_subvariant, design)?;
                    let parent = design.variant_registration(root)?;
                    if !parent.compounds_with().intersects(variant_compounds(&variant, design)) {
                        return None;
                    }
                    let modifier = match modifier_segment {
                        Some(raw) => Some(parse_modifier(raw)?),
                        None => None,
                    };
                    return Some(Self {
                        kind: VariantKind::Compound {
                            root: CompactString::new(root),
                            variant: Box::new(variant),
                            modifier,
                        },
                    });
                }
            }
        }
        None
    }

    pub fn print(&self) -> String {
        match &self.kind {
            VariantKind::Static { root } => root.to_string(),
            VariantKind::Arbitrary { selector, .. } => {
                let selector = selector
                    .strip_prefix("&:is(")
                    .and_then(|value| value.strip_suffix(')'))
                    .unwrap_or(selector);
                format!("[{}]", print_arbitrary(selector))
            }
            VariantKind::Functional { root, value, modifier } => {
                let mut output = root.to_string();
                if let Some(value) = value {
                    let dash = if root == "@" { "" } else { "-" };
                    output.push_str(dash);
                    match value {
                        VariantValue::Named(value) => output.push_str(value),
                        VariantValue::Arbitrary(value) => {
                            if let Some(variable) = css_variable(value) {
                                output.push('(');
                                output.push_str(&print_arbitrary(variable));
                                output.push(')');
                            } else {
                                output.push('[');
                                output.push_str(&print_arbitrary(value));
                                output.push(']');
                            }
                        }
                    }
                }
                output.push_str(&print_modifier(modifier.as_ref()));
                output
            }
            VariantKind::Compound { root, variant, modifier } => {
                let mut output = format!("{root}-{}", variant.print());
                output.push_str(&print_modifier(modifier.as_ref()));
                output
            }
        }
    }
}

fn variant_compounds(variant: &Variant, design: &DesignSystem) -> crate::Compounds {
    match &variant.kind {
        VariantKind::Arbitrary { selector, .. } => {
            if selector.starts_with('@') {
                crate::Compounds::AT_RULES
            } else {
                crate::Compounds::STYLE_RULES
            }
        }
        VariantKind::Static { root }
        | VariantKind::Functional { root, .. }
        | VariantKind::Compound { root, .. } => design
            .variant_registration(root)
            .map_or_else(crate::Compounds::empty, VariantRegistration::compounds),
    }
}

fn parse_modifier(input: &str) -> Option<Modifier> {
    if let Some(value) = input.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
        let value = decode_arbitrary(value);
        if value.trim().is_empty() || !is_valid_arbitrary(&value) {
            return None;
        }
        return Some(Modifier::Arbitrary(CompactString::new(value)));
    }
    if let Some(value) = input.strip_prefix('(').and_then(|value| value.strip_suffix(')')) {
        if !value.starts_with("--") || !is_valid_arbitrary(value) {
            return None;
        }
        return Some(Modifier::Arbitrary(CompactString::new(format!("var({value})"))));
    }
    is_valid_named(input).then(|| Modifier::Named(CompactString::new(input)))
}

fn parse_candidate_value(
    input: &str,
    modifier_segment: Option<&str>,
    modifier: Option<&Modifier>,
) -> Option<CandidateValue> {
    if let Some(body) = input.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
        let mut value = decode_arbitrary(body);
        if value.trim().is_empty() || !is_valid_arbitrary(&value) {
            return None;
        }
        let data_type = if let Some((hint, rest)) = type_hint(&value) {
            let hint = CompactString::new(hint);
            let rest = rest.to_owned();
            value = rest;
            Some(hint)
        } else {
            None
        };
        if value.trim().is_empty() {
            return None;
        }
        return Some(CandidateValue::Arbitrary(ArbitraryValue {
            data_type,
            value: CompactString::new(value),
        }));
    }
    if let Some(body) = input.strip_prefix('(').and_then(|value| value.strip_suffix(')')) {
        let parts = segment(body, b':');
        let (data_type, variable) = match parts.as_slice() {
            [variable] => (None, *variable),
            [data_type, variable] => (Some(CompactString::new(*data_type)), *variable),
            _ => return None,
        };
        if !variable.starts_with("--") || !is_valid_arbitrary(variable) {
            return None;
        }
        return Some(CandidateValue::Arbitrary(ArbitraryValue {
            data_type,
            value: CompactString::new(format!("var({variable})")),
        }));
    }
    if !is_valid_named(input) {
        return None;
    }
    let fraction = match (modifier_segment, modifier) {
        (Some(modifier_segment), Some(Modifier::Named(_))) => {
            Some(CompactString::new(format!("{input}/{modifier_segment}")))
        }
        _ => None,
    };
    Some(CandidateValue::Named(NamedValue { value: CompactString::new(input), fraction }))
}

fn parse_variant_value(input: &str) -> Option<VariantValue> {
    if let Some(value) = input.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
        let value = decode_arbitrary(value);
        if value.trim().is_empty() || !is_valid_arbitrary(&value) {
            return None;
        }
        return Some(VariantValue::Arbitrary(CompactString::new(value)));
    }
    if let Some(value) = input.strip_prefix('(').and_then(|value| value.strip_suffix(')')) {
        let value = decode_arbitrary(value);
        if !value.starts_with("--") || !is_valid_arbitrary(&value) {
            return None;
        }
        return Some(VariantValue::Arbitrary(CompactString::new(format!("var({value})"))));
    }
    is_valid_named(input).then(|| VariantValue::Named(CompactString::new(input)))
}

fn find_utility_roots<'a>(
    input: &'a str,
    design: &DesignSystem,
) -> Vec<(&'a str, Option<&'a str>)> {
    if input.ends_with(']') {
        let Some(index) = input.find("-[") else { return Vec::new() };
        let root = &input[..index];
        return if design.has_functional_utility(root) {
            vec![(root, Some(&input[index + 1..]))]
        } else {
            Vec::new()
        };
    }
    if input.ends_with(')') {
        let Some(index) = input.find("-(") else { return Vec::new() };
        let root = &input[..index];
        return if design.has_functional_utility(root) {
            vec![(root, Some(&input[index + 1..]))]
        } else {
            Vec::new()
        };
    }
    find_roots(input, |root| design.has_functional_utility(root))
}

fn find_variant_roots<'a>(
    input: &'a str,
    design: &DesignSystem,
) -> Vec<(&'a str, Option<&'a str>)> {
    find_roots(input, |root| design.variant_registration(root).is_some())
}

fn find_roots(input: &str, exists: impl Fn(&str) -> bool) -> Vec<(&str, Option<&str>)> {
    let mut roots = Vec::new();
    if exists(input) {
        roots.push((input, None));
    }
    let mut search_end = input.len();
    while let Some(index) = input[..search_end].rfind('-') {
        if index == 0 {
            break;
        }
        let root = &input[..index];
        let value = &input[index + 1..];
        if exists(root) {
            if value.is_empty() || root == "@" {
                break;
            }
            roots.push((root, Some(value)));
        }
        search_end = index;
    }
    if input.starts_with('@') && exists("@") {
        roots.push(("@", Some(&input[1..])));
    }
    roots
}

fn type_hint(value: &str) -> Option<(&str, &str)> {
    for (index, byte) in value.bytes().enumerate() {
        if byte == b':' {
            return (index > 0).then(|| (&value[..index], &value[index + 1..]));
        }
        if byte != b'-' && !byte.is_ascii_lowercase() {
            return None;
        }
    }
    None
}

fn is_valid_named(input: &str) -> bool {
    !input.is_empty()
        && input
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'%' | b'-'))
}

fn css_variable(value: &str) -> Option<&str> {
    value.strip_prefix("var(").and_then(|value| value.strip_suffix(')'))
}

fn print_modifier(modifier: Option<&Modifier>) -> String {
    let Some(modifier) = modifier else { return String::new() };
    match modifier {
        Modifier::Named(value) => format!("/{value}"),
        Modifier::Arbitrary(value) => css_variable(value).map_or_else(
            || format!("/[{}]", print_arbitrary(value)),
            |value| format!("/({})", print_arbitrary(value)),
        ),
    }
}

fn print_arbitrary(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '_' => output.push_str(r"\_"),
            ' ' => output.push('_'),
            _ => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{CandidateKind, CandidateValue, VariantKind};
    use crate::{DesignSystem, LoadOptions};

    fn design() -> DesignSystem {
        DesignSystem::load(
            &LoadOptions::new(env!("CARGO_MANIFEST_DIR"))
                .with_entry_point("tests/fixtures/base.css"),
            1,
        )
        .expect("fixture should load")
    }

    #[test]
    fn parses_and_prints_candidates() {
        let design = design();
        for (input, expected) in [
            ("block", "block"),
            ("hover:focus:bg-red-500/50!", "hover:focus:bg-red-500/50!"),
            ("bg-[color:var(--brand)]", "bg-(color:--brand)"),
            ("w-(--sidebar)", "w-(--sidebar)"),
            ("[color:red]", "[color:red]"),
            ("group-hover:block", "group-hover:block"),
        ] {
            let candidates = design.parse_candidate(input);
            assert!(!candidates.is_empty(), "failed to parse {input}");
            assert_eq!(candidates[0].print(&design), expected);
        }
    }

    #[test]
    fn preserves_candidate_shape() {
        let design = design();
        let candidate = design.parse_candidate("w-1/2").pop().expect("candidate");
        let CandidateKind::Functional { value: Some(CandidateValue::Named(value)), .. } =
            candidate.kind
        else {
            panic!("expected functional named candidate");
        };
        assert_eq!(value.value, "1");
        assert_eq!(value.fraction.as_deref(), Some("1/2"));
    }

    #[test]
    fn rejects_invalid_arbitrary_values() {
        let design = design();
        assert!(design.parse_candidate("bg-[]").is_empty());
        assert!(design.parse_candidate("[Color:red]").is_empty());
        assert!(design.parse_variant("[]").is_none());
    }

    #[test]
    fn rejects_invalid_candidate_shapes() {
        let design = design();
        assert!(!design.is_known_class("flex/foo"));
        for input in [
            "flex/foo/bar",
            "bg-red-500/50/25",
            "bg-[#0088cc",
            "bg-(my-color)",
            "[@media(width>=123px){&:hover}]:flex",
        ] {
            assert!(design.parse_candidate(input).is_empty(), "unexpectedly parsed {input}");
        }
    }

    #[test]
    fn parses_tailwind_v4_candidate_forms() {
        let design = design();
        for input in [
            "-translate-x-4",
            "[&_p]:flex",
            "supports-(--test):flex",
            "data-[disabled]:flex",
            "group-[&_p]/parent-name:flex",
            "bg-[#0088cc]",
            "bg-[color:var(--value)]",
            "bg-(color:--my-color)",
            "bg-red-500/[50%]",
        ] {
            assert!(!design.parse_candidate(input).is_empty(), "failed to parse {input}");
        }
    }

    #[test]
    fn parses_compound_variants() {
        let design = design();
        let variant = design.parse_variant("not-group-hover/name").expect("compound variant");
        let VariantKind::Compound { root, variant, modifier } = variant.kind else {
            panic!("expected compound variant");
        };
        assert_eq!(root, "not");
        assert!(modifier.is_none());
        assert!(matches!(variant.kind, VariantKind::Compound { .. }));
        assert!(design.parse_variant("group-sm").is_none());
        assert!(design.parse_variant("not-sm").is_some());
    }
}
