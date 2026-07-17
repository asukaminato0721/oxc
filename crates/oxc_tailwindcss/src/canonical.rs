use compact_str::CompactString;

use crate::{
    ArbitraryValue, Candidate, CandidateKind, CandidateValue, DesignSystem, Modifier, Variant,
    VariantKind, VariantValue, segment::segment,
};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CanonicalizeOptions {
    pub rem: Option<f64>,
    pub collapse: bool,
    pub logical_to_physical: bool,
}

impl DesignSystem {
    pub fn canonicalize_classes(
        &self,
        classes: &[&str],
        options: CanonicalizeOptions,
    ) -> Vec<CompactString> {
        let mut result = Vec::with_capacity(classes.len());
        for class_name in classes {
            let canonical = self.canonicalize_class(class_name, options);
            if !result.contains(&canonical) {
                result.push(canonical);
            }
        }
        if options.collapse && result.len() > 1 {
            collapse_classes(self, &mut result, options.logical_to_physical);
        }
        result
    }

    fn canonicalize_class(&self, class_name: &str, options: CanonicalizeOptions) -> CompactString {
        if let Some(class_name) = canonicalize_deprecated(self, class_name) {
            return CompactString::new(class_name);
        }
        if !self.is_known_class(class_name) {
            return CompactString::new(class_name);
        }
        let Some(mut candidate) = self.parse_candidate(class_name).into_iter().next() else {
            return CompactString::new(class_name);
        };
        for variant in &mut candidate.variants {
            canonicalize_variant(self, variant);
        }
        canonicalize_candidate_kind(self, &mut candidate, options);
        CompactString::new(candidate.print(self))
    }
}

fn canonicalize_variant(design: &DesignSystem, variant: &mut Variant) {
    if let VariantKind::Compound { variant, .. } = &mut variant.kind {
        canonicalize_variant(design, variant);
        return;
    }
    if let VariantKind::Arbitrary { selector, .. } = &variant.kind {
        if let Some(canonical) = canonical_arbitrary_variant(selector)
            && let Some(parsed) = design.parse_variant(&canonical)
        {
            *variant = parsed;
        }
        return;
    }
    if let VariantKind::Functional { root, value: Some(value), .. } = &mut variant.kind {
        if let VariantValue::Arbitrary(arbitrary) = value {
            *arbitrary = CompactString::new(rewrite_theme_functions(design, arbitrary, true));
        }
        let canonical = match (root.as_str(), &*value) {
            ("data" | "supports", VariantValue::Arbitrary(value))
                if is_simple_variant_value(value) =>
            {
                Some(VariantValue::Named(value.clone()))
            }
            ("aria", VariantValue::Arbitrary(value)) => value
                .strip_suffix("=\"true\"")
                .filter(|name| is_simple_variant_value(name))
                .map(|name| VariantValue::Named(CompactString::new(name))),
            _ => None,
        };
        if let Some(canonical) = canonical {
            *value = canonical;
        }
    }
}

fn canonical_arbitrary_variant(selector: &str) -> Option<String> {
    let compact =
        selector.chars().filter(|character| !character.is_ascii_whitespace()).collect::<String>();
    if let Some(root) = canonical_pseudo_variant(&compact) {
        return Some(root);
    }
    if let Some(attribute) = canonical_attribute_variant(&compact) {
        return Some(attribute);
    }
    let media = compact.strip_prefix("@media")?;
    let (negated, query) = media.strip_prefix("not").map_or((false, media), |query| (true, query));
    let root = match query {
        "print" => Some("print"),
        "(scripting:none)" => Some("noscript"),
        "(prefers-color-scheme:dark)" => Some("dark"),
        "(orientation:portrait)" => Some("portrait"),
        "(orientation:landscape)" => Some("landscape"),
        "(pointer:none)" => Some("pointer-none"),
        "(pointer:coarse)" => Some("pointer-coarse"),
        "(pointer:fine)" => Some("pointer-fine"),
        "(any-pointer:none)" => Some("any-pointer-none"),
        "(any-pointer:coarse)" => Some("any-pointer-coarse"),
        "(any-pointer:fine)" => Some("any-pointer-fine"),
        _ => None,
    };
    if let Some(root) = root {
        return Some(if negated { format!("not-{root}") } else { root.to_owned() });
    }
    if negated {
        let positive = format!("@media{}", selector.strip_prefix("@media not")?);
        return Some(format!("not-[{positive}]"));
    }
    None
}

fn canonical_pseudo_variant(selector: &str) -> Option<String> {
    let (negated, selector) = selector
        .strip_prefix("&:not(")
        .and_then(|selector| selector.strip_suffix(')'))
        .map_or((false, selector), |selector| (true, selector));
    let selector =
        selector.strip_prefix("&:").or_else(|| selector.strip_prefix(':')).unwrap_or(selector);
    let root = match selector {
        "focus" => Some("focus".to_owned()),
        "focus-visible" => Some("focus-visible".to_owned()),
        "focus-within" => Some("focus-within".to_owned()),
        "active" => Some("active".to_owned()),
        "visited" => Some("visited".to_owned()),
        "target" => Some("target".to_owned()),
        "first-child" => Some("first".to_owned()),
        "last-child" => Some("last".to_owned()),
        "only-child" => Some("only".to_owned()),
        "first-of-type" => Some("first-of-type".to_owned()),
        "last-of-type" => Some("last-of-type".to_owned()),
        "only-of-type" => Some("only-of-type".to_owned()),
        "empty" => Some("empty".to_owned()),
        "checked" => Some("checked".to_owned()),
        "disabled" => Some("disabled".to_owned()),
        "enabled" => Some("enabled".to_owned()),
        "required" => Some("required".to_owned()),
        "optional" => Some("optional".to_owned()),
        "valid" => Some("valid".to_owned()),
        "invalid" => Some("invalid".to_owned()),
        "nth-child(odd)" => Some("odd".to_owned()),
        "nth-child(even)" => Some("even".to_owned()),
        _ => canonical_nth_variant(selector),
    }?;
    Some(if negated {
        match root.as_str() {
            "odd" => "even".to_owned(),
            "even" => "odd".to_owned(),
            _ => format!("not-{root}"),
        }
    } else {
        root
    })
}

fn canonical_nth_variant(selector: &str) -> Option<String> {
    let (root, value) = if let Some(value) =
        selector.strip_prefix("nth-child(").and_then(|value| value.strip_suffix(')'))
    {
        ("nth", value)
    } else if let Some(value) =
        selector.strip_prefix("nth-last-child(").and_then(|value| value.strip_suffix(')'))
    {
        ("nth-last", value)
    } else {
        return None;
    };
    Some(if value.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{root}-{value}")
    } else {
        format!("{root}-[{value}]")
    })
}

fn canonical_attribute_variant(selector: &str) -> Option<String> {
    let selector = selector
        .strip_prefix("&:is(")
        .and_then(|selector| selector.strip_suffix(')'))
        .unwrap_or(selector);
    let selector = selector.strip_prefix('&').unwrap_or(selector);
    let attribute = selector.strip_prefix('[')?.strip_suffix(']')?;
    if let Some(data) = attribute.strip_prefix("data-") {
        return Some(if is_simple_variant_value(data) {
            format!("data-{data}")
        } else {
            format!("data-[{data}]")
        });
    }
    if let Some(aria) = attribute.strip_prefix("aria-") {
        if let Some(name) =
            aria.strip_suffix("=\"true\"").filter(|name| is_simple_variant_value(name))
        {
            return Some(format!("aria-{name}"));
        }
        return Some(format!("aria-[{aria}]"));
    }
    None
}

fn is_simple_variant_value(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn canonicalize_candidate_kind(
    design: &DesignSystem,
    candidate: &mut Candidate,
    options: CanonicalizeOptions,
) {
    if let CandidateKind::Arbitrary { property, value, modifier } = &candidate.kind {
        let value = value.trim();
        if let Some(root) = static_equivalent(property, value) {
            candidate.kind = CandidateKind::Static { root: CompactString::new(root) };
            return;
        }
        if let Some(root) = property_utility(property) {
            let modifier = normalize_modifier(modifier.clone());
            candidate.kind = CandidateKind::Functional {
                root: CompactString::new(root),
                value: Some(CandidateValue::Arbitrary(ArbitraryValue {
                    data_type: None,
                    value: CompactString::new(value),
                })),
                modifier,
            };
        }
    }

    let CandidateKind::Functional { root, value, modifier } = &mut candidate.kind else { return };
    *modifier = normalize_modifier(modifier.take());
    if let Some(CandidateValue::Arbitrary(arbitrary)) = value {
        arbitrary.value = CompactString::new(rewrite_arbitrary_value(design, &arbitrary.value));
    }
    if root == "bg"
        && let Some(CandidateValue::Arbitrary(arbitrary)) = value
    {
        if let Some(data_type) = arbitrary.data_type.as_deref()
            && matches!(data_type, "position" | "size")
        {
            *root =
                CompactString::new(if data_type == "position" { "bg-position" } else { "bg-size" });
            arbitrary.data_type = None;
        } else if arbitrary.data_type.is_none() && is_css_dimension(&arbitrary.value) {
            *root = CompactString::new("bg-position");
        }
    }
    let aspect_fraction = if root == "aspect" {
        value.as_ref().and_then(|value| match value {
            CandidateValue::Arbitrary(arbitrary) => arbitrary
                .value
                .split_once('/')
                .filter(|(left, right)| valid_fraction_part(left) && valid_fraction_part(right))
                .map(|(left, right)| {
                    (CompactString::new(left), CompactString::new(right), arbitrary.value.clone())
                }),
            CandidateValue::Named(_) => None,
        })
    } else {
        None
    };
    if let Some((left, right, fraction)) = aspect_fraction {
        *value = Some(CandidateValue::Named(crate::NamedValue {
            value: left,
            fraction: Some(fraction),
        }));
        *modifier = Some(Modifier::Named(right));
        return;
    }
    let grid_track_count = if matches!(root.as_str(), "grid-cols" | "grid-rows") {
        value.as_ref().and_then(|value| match value {
            CandidateValue::Arbitrary(arbitrary) => {
                canonical_grid_track_count(&arbitrary.value).map(CompactString::new)
            }
            CandidateValue::Named(_) => None,
        })
    } else {
        None
    };
    if let Some(value_name) = grid_track_count {
        *value =
            Some(CandidateValue::Named(crate::NamedValue { value: value_name, fraction: None }));
        return;
    }
    let bare_numeric = if matches!(root.as_str(), "col" | "row" | "grid-cols" | "grid-rows") {
        value.as_ref().and_then(|value| match value {
            CandidateValue::Arbitrary(arbitrary) if arbitrary.value.parse::<u32>().is_ok() => {
                Some(arbitrary.value.clone())
            }
            CandidateValue::Arbitrary(_) | CandidateValue::Named(_) => None,
        })
    } else {
        None
    };
    if let Some(value_name) = bare_numeric {
        *value =
            Some(CandidateValue::Named(crate::NamedValue { value: value_name, fraction: None }));
        return;
    }
    let bare_pixel = if matches!(
        root.as_str(),
        "border"
            | "border-x"
            | "border-y"
            | "border-t"
            | "border-r"
            | "border-b"
            | "border-l"
            | "divide-x"
            | "divide-y"
            | "outline"
            | "ring"
            | "decoration"
    ) {
        value.as_ref().and_then(|value| match value {
            CandidateValue::Arbitrary(arbitrary) => canonical_pixel_value(&arbitrary.value),
            CandidateValue::Named(_) => None,
        })
    } else {
        None
    };
    if let Some(value_name) = bare_pixel {
        *value = Some(CandidateValue::Named(crate::NamedValue {
            value: CompactString::new(value_name),
            fraction: None,
        }));
        return;
    }
    let bare_percentage = if matches!(root.as_str(), "from" | "via" | "to") {
        value.as_ref().and_then(|value| match value {
            CandidateValue::Arbitrary(arbitrary) => canonical_percentage(&arbitrary.value),
            CandidateValue::Named(_) => None,
        })
    } else {
        None
    };
    if let Some(value_name) = bare_percentage {
        *value = Some(CandidateValue::Named(crate::NamedValue {
            value: CompactString::new(value_name),
            fraction: None,
        }));
        return;
    }
    let legacy_spacing = if is_spacing_utility(root) && design.theme_value("--spacing").is_some() {
        value.as_ref().and_then(|value| match value {
            CandidateValue::Arbitrary(arbitrary) => {
                legacy_spacing_multiplier(&arbitrary.value).map(CompactString::new)
            }
            CandidateValue::Named(_) => None,
        })
    } else {
        None
    };
    if let Some(value_name) = legacy_spacing {
        *value =
            Some(CandidateValue::Named(crate::NamedValue { value: value_name, fraction: None }));
        return;
    }
    let Some(CandidateValue::Arbitrary(arbitrary)) = value else { return };
    if let Some(variable) = negated_variable(&arbitrary.value) {
        let was_negative = root.starts_with('-');
        *root = if was_negative {
            CompactString::new(root.strip_prefix('-').unwrap_or(root))
        } else {
            CompactString::new(format!("-{root}"))
        };
        arbitrary.value = CompactString::new(format!("var({variable})"));
    }
    if let Some(positive_root) = root.strip_prefix('-') {
        if arbitrary.value.starts_with("var(") {
            return;
        }
        if is_spacing_utility(positive_root) {
            let (double_negative, magnitude) = arbitrary
                .value
                .strip_prefix('-')
                .map_or((false, arbitrary.value.as_str()), |value| (true, value));
            if let Some(multiplier) = spacing_multiplier(design, magnitude, options.rem)
                .or_else(|| spacing_expression_multiplier(magnitude))
            {
                if double_negative {
                    *root = CompactString::new(positive_root);
                }
                *value = Some(CandidateValue::Named(crate::NamedValue {
                    value: CompactString::new(format_number(multiplier)),
                    fraction: None,
                }));
                return;
            }
        }
    }
    if let Some(positive_root) = root.strip_prefix('-') {
        *root = CompactString::new(positive_root);
        arbitrary.value = CompactString::new(negate_value(&arbitrary.value));
    }
    if let Some(variable) = legacy_theme_variable(design, &arbitrary.value) {
        arbitrary.value = CompactString::new(format!("var({variable})"));
    }
    if let Some(named) = theme_name_for_value(design, root, &arbitrary.value, options.rem) {
        *value = Some(CandidateValue::Named(crate::NamedValue {
            value: CompactString::new(named),
            fraction: None,
        }));
        return;
    }
    if let Some(multiplier) = spacing_multiplier(design, &arbitrary.value, options.rem)
        .or_else(|| spacing_expression_multiplier(&arbitrary.value))
        && is_spacing_utility(root)
    {
        *value = Some(CandidateValue::Named(crate::NamedValue {
            value: CompactString::new(format_number(multiplier)),
            fraction: None,
        }));
    }
}

fn canonical_grid_track_count(value: &str) -> Option<&str> {
    let body = value.strip_prefix("repeat(")?.strip_suffix(')')?;
    let parts = segment(body, b',');
    let [count, track] = parts.as_slice() else { return None };
    let count = count.trim();
    (count.parse::<u32>().is_ok_and(|count| count > 0)
        && track
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .eq("minmax(0,1fr)".chars()))
    .then_some(count)
}

fn canonical_pixel_value(value: &str) -> Option<&str> {
    let value = value.strip_suffix("px")?;
    let number = value.parse::<f64>().ok()?;
    (number.is_finite() && number >= 0.0 && number.fract() == 0.0).then_some(value)
}

fn canonical_percentage(value: &str) -> Option<&str> {
    let number = value.strip_suffix('%')?.parse::<f64>().ok()?;
    (number.is_finite() && number >= 0.0 && number.fract() == 0.0).then_some(value)
}

fn spacing_expression_multiplier(value: &str) -> Option<f64> {
    if let Some(value) = value.strip_prefix("--spacing(").and_then(|value| value.strip_suffix(')'))
    {
        return value.parse().ok();
    }
    let expression = value.strip_prefix("calc(")?.strip_suffix(')')?;
    let expression =
        expression.chars().filter(|character| !character.is_ascii_whitespace()).collect::<String>();
    expression
        .strip_prefix("var(--spacing)*")
        .or_else(|| expression.strip_suffix("*var(--spacing)"))?
        .parse()
        .ok()
}

fn rewrite_arbitrary_value(design: &DesignSystem, value: &str) -> String {
    let value = rewrite_theme_functions(design, value, false);
    let value = rewrite_spacing_calculations(&value);
    normalize_comma_whitespace(&value)
}

fn rewrite_theme_functions(design: &DesignSystem, input: &str, in_variant: bool) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_start) = input[cursor..].find("theme(") {
        let start = cursor + relative_start;
        let open = start + "theme".len();
        let Some(close) = matching_parenthesis(input, open) else { break };
        output.push_str(&input[cursor..start]);
        let path = input[open + 1..close].trim();
        if let Some(variable) = legacy_theme_path(design, path) {
            if let Some(multiplier) = variable.strip_prefix("--spacing(") {
                output.push_str("--spacing(");
                output.push_str(multiplier);
            } else if in_variant {
                output.push_str("--theme(");
                output.push_str(&variable);
                output.push(')');
            } else {
                output.push_str("var(");
                output.push_str(&variable);
                output.push(')');
            }
        } else {
            output.push_str(&input[start..=close]);
        }
        cursor = close + 1;
    }
    output.push_str(&input[cursor..]);
    output
}

fn legacy_theme_path(design: &DesignSystem, path: &str) -> Option<String> {
    if path.contains([',', '/']) {
        return None;
    }
    let path = normalize_legacy_theme_path(path);
    if let Some(value) = path.strip_prefix("spacing.") {
        let number = value.parse::<f64>().ok()?;
        return (number >= 0.0
            && number.is_finite()
            && (number * 4.0).fract() == 0.0
            && format_number(number) == value
            && design.theme_value("--spacing").is_some())
        .then(|| format!("--spacing({value})"));
    }
    let variable = if let Some(name) = path.strip_prefix("colors.") {
        format!("--color-{}", hyphenate_theme_path(name))
    } else if let Some(name) = path.strip_prefix("fontSize.") {
        let name = name.strip_suffix(".1.lineHeight").map_or_else(
            || hyphenate_theme_path(name),
            |name| format!("{}--line-height", hyphenate_theme_path(name)),
        );
        format!("--text-{name}")
    } else if let Some(name) = path.strip_prefix("fontFamily.") {
        format!("--font-{}", hyphenate_theme_path(name))
    } else if let Some(name) = path.strip_prefix("borderRadius.") {
        format!("--radius-{}", hyphenate_theme_path(name))
    } else if let Some(name) = path.strip_prefix("boxShadow.") {
        format!("--shadow-{}", hyphenate_theme_path(name))
    } else if let Some(name) = path.strip_prefix("screens.") {
        format!("--breakpoint-{}", hyphenate_theme_path(name))
    } else if let Some(name) = path.strip_prefix("maxWidth.") {
        format!("--container-{}", hyphenate_theme_path(name))
    } else {
        return None;
    };
    design.theme_value(&variable).map(|_| variable)
}

fn normalize_legacy_theme_path(path: &str) -> String {
    let mut output = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            '[' => output.push('.'),
            ']' => {}
            _ => output.push(character),
        }
    }
    output
}

fn hyphenate_theme_path(path: &str) -> String {
    path.chars().map(|character| if character == '.' { '-' } else { character }).collect()
}

fn matching_parenthesis(input: &str, open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (offset, character) in input[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn rewrite_spacing_calculations(input: &str) -> String {
    const PREFIX: &str = "calc(var(--spacing)*";
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_start) = input[cursor..].find(PREFIX) {
        let start = cursor + relative_start;
        let open = start + "calc".len();
        let Some(close) = matching_parenthesis(input, open) else { break };
        let argument_start = start + PREFIX.len();
        let argument = &input[argument_start..close];
        output.push_str(&input[cursor..start]);
        output.push_str("--spacing(");
        output.push_str(argument);
        output.push(')');
        cursor = close + 1;
    }
    output.push_str(&input[cursor..]);
    output
}

fn normalize_comma_whitespace(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;
    for character in input.chars() {
        if character.is_ascii_whitespace() {
            pending_space = true;
            continue;
        }
        if character == ',' {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push(',');
            pending_space = false;
            continue;
        }
        if pending_space && !output.is_empty() && !output.ends_with(',') {
            output.push(' ');
        }
        pending_space = false;
        output.push(character);
    }
    output
}

fn negated_variable(value: &str) -> Option<&str> {
    let expression = value.strip_prefix("calc(")?.strip_suffix(')')?;
    expression
        .strip_suffix("*-1")
        .and_then(|value| value.strip_prefix("var("))
        .and_then(|value| value.strip_suffix(')'))
        .or_else(|| expression.strip_prefix("-1*var(").and_then(|value| value.strip_suffix(')')))
        .filter(|variable| variable.starts_with("--"))
}

fn is_css_dimension(value: &str) -> bool {
    const UNITS: &[&str] = &[
        "px", "rem", "em", "%", "ch", "ex", "cap", "lh", "rlh", "vw", "vh", "vmin", "vmax", "svw",
        "svh", "lvw", "lvh", "dvw", "dvh", "cm", "mm", "in", "pt", "pc",
    ];
    UNITS
        .iter()
        .any(|unit| value.strip_suffix(unit).is_some_and(|number| number.parse::<f64>().is_ok()))
}

fn valid_fraction_part(value: &str) -> bool {
    value.parse::<f64>().is_ok_and(|value| value.is_finite() && value >= 0.0)
}

fn legacy_theme_variable(design: &DesignSystem, value: &str) -> Option<String> {
    let path = value.strip_prefix("theme(")?.strip_suffix(')')?.trim();
    if path.contains([',', '/']) {
        return None;
    }
    let (legacy_namespace, namespace) = [
        ("colors.", "--color-"),
        ("fontSize.", "--text-"),
        ("fontFamily.", "--font-"),
        ("borderRadius.", "--radius-"),
        ("boxShadow.", "--shadow-"),
    ]
    .into_iter()
    .find_map(|(legacy, namespace)| path.strip_prefix(legacy).map(|name| (name, namespace)))?;
    let mut variable = String::from(namespace);
    for character in legacy_namespace.chars() {
        variable.push(if character == '.' { '-' } else { character });
    }
    design.theme_value(&variable).map(|_| variable)
}

fn legacy_spacing_multiplier(value: &str) -> Option<&str> {
    let path = value.strip_prefix("theme(")?.strip_suffix(')')?.trim();
    let value = path.strip_prefix("spacing.")?;
    let number = value.parse::<f64>().ok()?;
    (number >= 0.0
        && number.is_finite()
        && (number * 4.0).fract() == 0.0
        && format_number(number) == value)
        .then_some(value)
}

fn negate_value(value: &str) -> String {
    value.strip_prefix('-').map_or_else(|| format!("-{value}"), ToOwned::to_owned)
}

fn canonicalize_deprecated(design: &DesignSystem, class_name: &str) -> Option<String> {
    let segments = segment(class_name, b':');
    let utility = *segments.last()?;
    let prefix_len = class_name.len() - utility.len();
    let prefix = &class_name[..prefix_len];
    let (utility, important) = if let Some(value) = utility.strip_suffix('!') {
        (value, "!")
    } else if let Some(value) = utility.strip_prefix('!') {
        (value, "!")
    } else {
        (utility, "")
    };
    if design.custom_utility_properties(utility).is_some() {
        return None;
    }

    let replacement = match utility {
        "bg-gradient-to-t" => "bg-linear-to-t",
        "bg-gradient-to-tr" => "bg-linear-to-tr",
        "bg-gradient-to-r" => "bg-linear-to-r",
        "bg-gradient-to-br" => "bg-linear-to-br",
        "bg-gradient-to-b" => "bg-linear-to-b",
        "bg-gradient-to-bl" => "bg-linear-to-bl",
        "bg-gradient-to-l" => "bg-linear-to-l",
        "bg-gradient-to-tl" => "bg-linear-to-tl",
        "order-none" => "order-0",
        "break-words" => "wrap-break-word",
        "overflow-ellipsis" => "text-ellipsis",
        _ => {
            let (negative, utility) =
                utility.strip_prefix('-').map_or((false, utility), |value| (true, value));
            let (root, value) = utility.split_once('-')?;
            let root = match root {
                "start" => "inset-s",
                "end" => "inset-e",
                _ => return None,
            };
            return Some(format!(
                "{prefix}{}{root}-{value}{important}",
                if negative { "-" } else { "" }
            ));
        }
    };
    Some(format!("{prefix}{replacement}{important}"))
}

fn normalize_modifier(modifier: Option<Modifier>) -> Option<Modifier> {
    match modifier {
        Some(Modifier::Named(value)) if matches!(value.as_str(), "100" | "100%") => None,
        Some(Modifier::Arbitrary(value)) => {
            let opacity =
                value.strip_suffix('%').and_then(|value| value.parse::<f64>().ok()).or_else(|| {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| (0.0..=1.0).contains(value))
                        .map(|value| value * 100.0)
                });
            if let Some(opacity) = opacity
                && opacity.is_finite()
                && opacity >= 0.0
                && (opacity * 4.0).fract() == 0.0
            {
                if (opacity - 100.0).abs() < f64::EPSILON {
                    return None;
                }
                return Some(Modifier::Named(CompactString::new(format_number(opacity))));
            }
            Some(Modifier::Arbitrary(value))
        }
        modifier => modifier,
    }
}

fn static_equivalent(property: &str, value: &str) -> Option<&'static str> {
    match (property, value) {
        ("display", "block") => Some("block"),
        ("display", "inline") => Some("inline"),
        ("display", "inline-block") => Some("inline-block"),
        ("display", "flex") => Some("flex"),
        ("display", "inline-flex") => Some("inline-flex"),
        ("display", "grid") => Some("grid"),
        ("display", "inline-grid") => Some("inline-grid"),
        ("display", "none") => Some("hidden"),
        ("position", "static") => Some("static"),
        ("position", "fixed") => Some("fixed"),
        ("position", "absolute") => Some("absolute"),
        ("position", "relative") => Some("relative"),
        ("position", "sticky") => Some("sticky"),
        ("text-decoration-line", "underline") => Some("underline"),
        ("text-decoration-line", "line-through") => Some("line-through"),
        ("text-wrap", "balance") => Some("text-balance"),
        ("border-style", "solid") => Some("border-solid"),
        ("border-style", "dashed") => Some("border-dashed"),
        ("border-style", "dotted") => Some("border-dotted"),
        ("border-style", "double") => Some("border-double"),
        ("border-style", "none") => Some("border-none"),
        _ => None,
    }
}

fn property_utility(property: &str) -> Option<&'static str> {
    match property {
        "color" | "font-size" => Some("text"),
        "line-height" => Some("leading"),
        "font-weight" => Some("font"),
        "background-color" => Some("bg"),
        "width" => Some("w"),
        "height" => Some("h"),
        "min-width" => Some("min-w"),
        "max-width" => Some("max-w"),
        "min-height" => Some("min-h"),
        "max-height" => Some("max-h"),
        "margin" => Some("m"),
        "margin-inline" => Some("mx"),
        "margin-block" => Some("my"),
        "margin-top" => Some("mt"),
        "margin-right" => Some("mr"),
        "margin-bottom" => Some("mb"),
        "margin-left" => Some("ml"),
        "padding" => Some("p"),
        "padding-inline" => Some("px"),
        "padding-block" => Some("py"),
        "padding-top" => Some("pt"),
        "padding-right" => Some("pr"),
        "padding-bottom" => Some("pb"),
        "padding-left" => Some("pl"),
        "border-radius" => Some("rounded"),
        "opacity" => Some("opacity"),
        "z-index" => Some("z"),
        "grid-column" => Some("col"),
        "grid-template-columns" => Some("grid-cols"),
        "grid-template-rows" => Some("grid-rows"),
        _ => None,
    }
}

fn theme_name_for_value<'a>(
    design: &'a DesignSystem,
    root: &str,
    value: &str,
    rem: Option<f64>,
) -> Option<&'a str> {
    let namespaces: &[&str] = match root {
        "text" if dimension_px(value, rem).is_some() => &["--text-"],
        "text" | "bg" | "fill" | "stroke" => &["--color-"],
        "rounded" => &["--radius-"],
        "shadow" => &["--shadow-"],
        "font" if value.parse::<f64>().is_ok() => &["--font-weight-"],
        "font" => &["--font-"],
        "leading" => &["--leading-"],
        "tracking" => &["--tracking-"],
        "w" | "min-w" | "max-w" => &["--width-", "--container-"],
        "h" | "min-h" | "max-h" => &["--height-"],
        _ => &[],
    };
    if let Some(variable) = value.strip_prefix("var(").and_then(|value| value.strip_suffix(')'))
        && let Some(variable) = design.theme.keys().find(|name| name.as_str() == variable)
        && let Some(name) = namespaces.iter().find_map(|namespace| variable.strip_prefix(namespace))
    {
        return Some(name);
    }
    design.theme.iter().find_map(|(name, theme_value)| {
        values_equivalent(theme_value, value, rem)
            .then(|| namespaces.iter().find_map(|namespace| name.strip_prefix(namespace)))
            .flatten()
    })
}

fn values_equivalent(left: &str, right: &str, rem: Option<f64>) -> bool {
    left == right
        || left.starts_with('#') && right.starts_with('#') && left.eq_ignore_ascii_case(right)
        || dimension_px(left, rem)
            .zip(dimension_px(right, rem))
            .is_some_and(|(left, right)| (left - right).abs() < f64::EPSILON)
}

fn spacing_multiplier(design: &DesignSystem, value: &str, rem: Option<f64>) -> Option<f64> {
    let spacing = design.theme_value("--spacing")?;
    let spacing_px = dimension_px(spacing, rem)?;
    let value_px = dimension_px(value, rem)?;
    let multiplier = value_px / spacing_px;
    (multiplier.is_finite() && multiplier >= 0.0 && (multiplier * 4.0).fract() == 0.0)
        .then_some(multiplier)
}

fn dimension_px(value: &str, rem: Option<f64>) -> Option<f64> {
    evaluate_dimension(value, rem).map(|dimension| dimension.value)
}

#[derive(Clone, Copy)]
struct EvaluatedDimension {
    value: f64,
    has_unit: bool,
}

fn evaluate_dimension(value: &str, rem: Option<f64>) -> Option<EvaluatedDimension> {
    let compact =
        value.chars().filter(|character| !character.is_ascii_whitespace()).collect::<String>();
    let value =
        compact.strip_prefix("calc(").and_then(|value| value.strip_suffix(')')).unwrap_or(&compact);
    if let Some((index, operator)) = top_level_operator(value, &['+', '-']) {
        let left = evaluate_dimension(&value[..index], rem)?;
        let right = evaluate_dimension(&value[index + operator.len_utf8()..], rem)?;
        if left.has_unit != right.has_unit && left.value != 0.0 && right.value != 0.0 {
            return None;
        }
        return Some(EvaluatedDimension {
            value: if operator == '+' {
                left.value + right.value
            } else {
                left.value - right.value
            },
            has_unit: left.has_unit || right.has_unit,
        });
    }
    if let Some((index, _)) = top_level_operator(value, &['*']) {
        let left = evaluate_dimension(&value[..index], rem)?;
        let right = evaluate_dimension(&value[index + 1..], rem)?;
        if left.has_unit && right.has_unit {
            return None;
        }
        return Some(EvaluatedDimension {
            value: left.value * right.value,
            has_unit: left.has_unit || right.has_unit,
        });
    }
    if let Some(value) = value.strip_suffix("px") {
        return Some(EvaluatedDimension { value: value.parse().ok()?, has_unit: true });
    }
    if let Some(value) = value.strip_suffix("rem") {
        return Some(EvaluatedDimension {
            value: value.parse::<f64>().ok()? * rem?,
            has_unit: true,
        });
    }
    Some(EvaluatedDimension { value: value.parse().ok()?, has_unit: false })
}

fn top_level_operator(value: &str, operators: &[char]) -> Option<(usize, char)> {
    let mut depth = 0_u32;
    let mut found = None;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            operator if depth == 0 && operators.contains(&operator) && index > 0 => {
                let previous = value[..index].chars().next_back()?;
                if !matches!(previous, '+' | '-' | '*' | '/') {
                    found = Some((index, operator));
                }
            }
            _ => {}
        }
    }
    found
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { value.to_string() }
}

fn is_spacing_utility(root: &str) -> bool {
    matches!(
        root,
        "m" | "mx"
            | "my"
            | "mt"
            | "mr"
            | "mb"
            | "ml"
            | "p"
            | "px"
            | "py"
            | "pt"
            | "pr"
            | "pb"
            | "pl"
            | "w"
            | "h"
            | "min-w"
            | "max-w"
            | "min-h"
            | "max-h"
            | "size"
            | "gap"
            | "gap-x"
            | "gap-y"
            | "leading"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "inset"
            | "inset-x"
            | "inset-y"
    )
}

fn collapse_classes(
    design: &DesignSystem,
    classes: &mut Vec<CompactString>,
    logical_to_physical: bool,
) {
    loop {
        let mut changed = false;
        if let Some((indices, replacement)) = find_text_leading_collapse(design, classes) {
            for index in indices.into_iter().rev() {
                classes.remove(index);
            }
            if !classes.iter().any(|class_name| class_name == replacement.as_str()) {
                classes.push(CompactString::new(replacement));
            }
            continue;
        }
        for pattern in collapse_patterns(logical_to_physical) {
            if let Some((indices, replacement)) = find_collapse(classes, pattern) {
                for index in indices.into_iter().rev() {
                    classes.remove(index);
                }
                if !classes.iter().any(|class_name| class_name == replacement.as_str()) {
                    classes.push(CompactString::new(replacement));
                }
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
}

fn find_text_leading_collapse(
    design: &DesignSystem,
    classes: &[CompactString],
) -> Option<([usize; 2], String)> {
    for (text_index, text) in classes.iter().enumerate() {
        let Some((prefix, text_value, important)) = class_parts(text, "text") else {
            continue;
        };
        if !design
            .compile_class(text)
            .iter()
            .flat_map(|rule| &rule.properties)
            .any(|property| property.css_property_name == "font-size")
        {
            continue;
        }
        let text_value = segment(text_value, b'/')[0];
        for (leading_index, leading) in classes.iter().enumerate() {
            if text_index == leading_index {
                continue;
            }
            let Some((leading_prefix, leading_value, leading_important)) =
                class_parts(leading, "leading")
            else {
                continue;
            };
            if prefix == leading_prefix && important == leading_important {
                let mut indices = [text_index, leading_index];
                indices.sort_unstable();
                return Some((
                    indices,
                    format!("{prefix}text-{text_value}/{leading_value}{important}"),
                ));
            }
        }
    }
    None
}

#[derive(Clone, Copy)]
struct CollapsePattern {
    inputs: &'static [&'static str],
    output: &'static str,
}

fn collapse_patterns(logical_to_physical: bool) -> Vec<CollapsePattern> {
    let mut patterns = vec![
        CollapsePattern { inputs: &["w", "h"], output: "size" },
        CollapsePattern { inputs: &["top", "right", "bottom", "left"], output: "inset" },
        CollapsePattern { inputs: &["inset-x", "inset-y"], output: "inset" },
        CollapsePattern { inputs: &["mt", "mr", "mb", "ml"], output: "m" },
        CollapsePattern { inputs: &["pt", "pr", "pb", "pl"], output: "p" },
        CollapsePattern { inputs: &["mx", "my"], output: "m" },
        CollapsePattern { inputs: &["px", "py"], output: "p" },
        CollapsePattern { inputs: &["ms", "me"], output: "mx" },
        CollapsePattern { inputs: &["ps", "pe"], output: "px" },
        CollapsePattern { inputs: &["mt", "mb"], output: "my" },
        CollapsePattern { inputs: &["pt", "pb"], output: "py" },
        CollapsePattern {
            inputs: &["border-t", "border-r", "border-b", "border-l"],
            output: "border",
        },
        CollapsePattern { inputs: &["border-x", "border-y"], output: "border" },
        CollapsePattern { inputs: &["border-t", "border-b"], output: "border-y" },
        CollapsePattern { inputs: &["border-l", "border-r"], output: "border-x" },
        CollapsePattern {
            inputs: &["scroll-mt", "scroll-mr", "scroll-mb", "scroll-ml"],
            output: "scroll-m",
        },
        CollapsePattern {
            inputs: &["scroll-pt", "scroll-pr", "scroll-pb", "scroll-pl"],
            output: "scroll-p",
        },
        CollapsePattern { inputs: &["scroll-mx", "scroll-my"], output: "scroll-m" },
        CollapsePattern { inputs: &["scroll-mt", "scroll-mb"], output: "scroll-my" },
        CollapsePattern { inputs: &["scroll-ms", "scroll-me"], output: "scroll-mx" },
        CollapsePattern { inputs: &["scroll-px", "scroll-py"], output: "scroll-p" },
        CollapsePattern { inputs: &["scroll-pt", "scroll-pb"], output: "scroll-py" },
        CollapsePattern { inputs: &["scroll-ps", "scroll-pe"], output: "scroll-px" },
        CollapsePattern { inputs: &["overflow-x", "overflow-y"], output: "overflow" },
        CollapsePattern { inputs: &["overscroll-x", "overscroll-y"], output: "overscroll" },
        CollapsePattern { inputs: &["gap-x", "gap-y"], output: "gap" },
        CollapsePattern { inputs: &["translate-x", "translate-y"], output: "translate" },
        CollapsePattern { inputs: &["rotate-x", "rotate-y"], output: "rotate" },
        CollapsePattern { inputs: &["scale-x", "scale-y"], output: "scale" },
    ];
    if logical_to_physical {
        patterns.push(CollapsePattern { inputs: &["ml", "mr"], output: "mx" });
        patterns.push(CollapsePattern { inputs: &["pl", "pr"], output: "px" });
    }
    patterns
}

fn find_collapse(
    classes: &[CompactString],
    pattern: CollapsePattern,
) -> Option<(Vec<usize>, String)> {
    for (first_index, first) in classes.iter().enumerate() {
        let Some((prefix, value, important)) = class_parts(first, pattern.inputs[0]) else {
            continue;
        };
        let mut indices = vec![first_index];
        for expected_root in &pattern.inputs[1..] {
            let index = classes.iter().enumerate().find_map(|(index, candidate)| {
                (!indices.contains(&index))
                    .then(|| class_parts(candidate, expected_root))
                    .flatten()
                    .filter(|parts| parts.0 == prefix && parts.1 == value && parts.2 == important)
                    .map(|_| index)
            })?;
            indices.push(index);
        }
        indices.sort_unstable();
        let separator = if value == "1" && pattern.output.starts_with("border") { "" } else { "-" };
        let value = if separator.is_empty() { "" } else { value };
        return Some((indices, format!("{prefix}{}{separator}{value}{important}", pattern.output)));
    }
    None
}

fn class_parts<'a>(
    class_name: &'a str,
    expected_root: &str,
) -> Option<(&'a str, &'a str, &'a str)> {
    let segments = segment(class_name, b':');
    let utility = segments.last()?;
    let prefix_len = class_name.len() - utility.len();
    let prefix = &class_name[..prefix_len];
    let (utility, important) =
        utility.strip_suffix('!').map_or((*utility, ""), |value| (value, "!"));
    let value = utility.strip_prefix(expected_root)?.strip_prefix('-')?;
    (!value.is_empty()).then_some((prefix, value, important))
}

#[cfg(test)]
mod tests {
    use crate::{CanonicalizeOptions, DesignSystem, LoadOptions};

    fn design() -> DesignSystem {
        DesignSystem::load(
            &LoadOptions::new(env!("CARGO_MANIFEST_DIR"))
                .with_entry_point("tests/fixtures/base.css"),
            1,
        )
        .expect("fixture should load")
    }

    #[test]
    fn canonicalizes_arbitrary_properties() {
        let design = design();
        assert_eq!(
            design.canonicalize_classes(
                &["[display:flex]", "[color:red]/100"],
                CanonicalizeOptions::default()
            ),
            ["flex", "text-[red]"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &[
                    "[display:_flex_]",
                    "[color:var(--color-red-500)]",
                    "[font-weight:400]",
                    "[grid-column:2]",
                ],
                CanonicalizeOptions::default()
            ),
            ["flex", "text-red-500", "font-normal", "col-2"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["bg-[theme(colors.red.500)]", "pt-[theme(spacing.4)]"],
                CanonicalizeOptions::default()
            ),
            ["bg-red-500", "pt-4"]
        );
    }

    #[test]
    fn canonicalizes_variants() {
        let design = design();
        assert_eq!(
            design.canonicalize_classes(&["[@media_print]:flex"], CanonicalizeOptions::default()),
            ["print:flex"]
        );
    }

    #[test]
    fn collapses_matching_utilities() {
        let design = design();
        let options = CanonicalizeOptions {
            collapse: true,
            logical_to_physical: true,
            ..CanonicalizeOptions::default()
        };
        assert_eq!(design.canonicalize_classes(&["w-4", "h-4"], options), ["size-4"]);
        assert_eq!(
            design.canonicalize_classes(&["top-0", "right-0", "bottom-0", "left-0"], options),
            ["inset-0"]
        );
        assert_eq!(
            design.canonicalize_classes(&["underline", "h-4", "w-4", "text-sm"], options),
            ["underline", "text-sm", "size-4"]
        );
    }

    #[test]
    fn canonicalizes_deprecated_and_negative_utilities() {
        let design = design();
        assert_eq!(
            design.canonicalize_classes(
                &["bg-gradient-to-r", "hover:-start-8", "!order-none"],
                CanonicalizeOptions::default()
            ),
            ["bg-linear-to-r", "hover:-inset-s-8", "order-0!"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["-mt-[0.04in]", "-mt-[-0.04in]"],
                CanonicalizeOptions::default()
            ),
            ["mt-[-0.04in]", "mt-[0.04in]"]
        );
        assert_eq!(
            design.canonicalize_classes(&["aspect-[4/3]"], CanonicalizeOptions::default()),
            ["aspect-4/3"]
        );
        assert_eq!(
            design.canonicalize_classes(&["bg-[#3F3CBB]"], CanonicalizeOptions::default()),
            ["bg-brand-purple"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["bg-red-500/[25%]", "bg-[#f00]/[0.16]", "bg-red-500/100"],
                CanonicalizeOptions::default()
            ),
            ["bg-red-500/25", "bg-[#f00]/16", "bg-red-500"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["px-[calc(1rem+0px)]"],
                CanonicalizeOptions { rem: Some(16.0), ..CanonicalizeOptions::default() }
            ),
            ["px-4"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["[font-size:14px]"],
                CanonicalizeOptions { rem: Some(16.0), ..CanonicalizeOptions::default() }
            ),
            ["text-sm"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["[font-size:14px]", "[line-height:28px]"],
                CanonicalizeOptions { rem: Some(16.0), collapse: true, logical_to_physical: true }
            ),
            ["text-sm/7"]
        );
        assert_eq!(
            design.canonicalize_classes(
                &["px-[1.2rem]", "py-[1.2rem]", "text-left"],
                CanonicalizeOptions { rem: Some(16.0), collapse: true, logical_to_physical: true }
            ),
            ["text-left", "p-[1.2rem]"]
        );
    }

    #[test]
    fn canonicalizes_upstream_tailwind_fixtures() {
        let design = design();
        let options =
            CanonicalizeOptions { rem: Some(16.0), collapse: true, logical_to_physical: true };

        for (input, expected) in [
            ("[text-wrap:balance]", "text-balance"),
            ("[color:#FFF]", "text-white"),
            ("[background-color:var(--color-red-500)]", "bg-red-500"),
            ("[max-height:20%]", "max-h-[20%]"),
            ("[grid-template-columns:repeat(2,minmax(0,1fr))]", "grid-cols-2"),
            ("leading-[1]", "leading-none"),
            ("border-[2px]", "border-2"),
            ("bg-[position:123px]", "bg-position-[123px]"),
            ("bg-[size:123px]", "bg-size-[123px]"),
            ("from-[25%]", "from-25%"),
            ("w-[64rem]", "w-256"),
            ("-mt-[12rem]", "-mt-48"),
            ("-mt-[-12rem]", "mt-48"),
            ("-mt-[var(--my-var)]", "-mt-(--my-var)"),
            ("bg-[theme(colors.red.500)]", "bg-red-500"),
            ("pt-[calc(var(--spacing)*8)]", "pt-8"),
        ] {
            assert_eq!(
                design.canonicalize_classes(&[input], options),
                [expected],
                "canonicalization mismatch for {input}"
            );
        }
    }
}
