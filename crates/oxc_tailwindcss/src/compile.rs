use std::{cmp::Ordering, sync::Arc};

use compact_str::CompactString;
use rustc_hash::FxHashMap;

use crate::{
    Candidate, CandidateKind, CandidateValue, DesignSystem, Modifier, Variant, VariantKind,
    VariantRegistration, VariantValue,
    design_system::{
        ARBITRARY_VARIANT_ORDER, MAX_VARIANT_ORDER, THEME_VARIANT_ORDER, THEME_VARIANT_RANGE,
    },
    property_order::property_rank,
};

const MAX_CACHED_CLASSES: usize = 4096;

macro_rules! properties {
    ($($name:expr),* $(,)?) => {
        vec![$(CompactString::new($name)),*]
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub css_property_name: CompactString,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRule {
    pub context: CompactString,
    pub properties: Vec<Property>,
    sort_property: Option<CompactString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassConflict {
    pub class_name: CompactString,
    pub conflicting_class_name: CompactString,
    pub properties: Vec<Property>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SortKey {
    variants: Vec<u64>,
    property_order: Vec<u16>,
    property_count: usize,
    candidate: CompactString,
}

impl DesignSystem {
    pub fn compile_class(&self, class_name: &str) -> Arc<[CompiledRule]> {
        if let Some(rules) = self
            .compiled_cache
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(class_name)
        {
            return Arc::clone(rules);
        }

        let mut compiled = self
            .cached_candidates(class_name)
            .iter()
            .filter_map(|candidate| self.compile_candidate(candidate))
            .collect::<Vec<_>>();
        compiled.dedup();
        let rules: Arc<[CompiledRule]> = compiled.into();
        let mut cache =
            self.compiled_cache.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= MAX_CACHED_CLASSES {
            cache.clear();
        }
        Arc::clone(cache.entry(CompactString::new(class_name)).or_insert(rules))
    }

    pub fn is_known_class(&self, class_name: &str) -> bool {
        !self.compile_class(class_name).is_empty()
    }

    pub fn unknown_classes<'a>(&self, classes: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
        classes.into_iter().filter(|class_name| !self.is_known_class(class_name)).collect()
    }

    /// Return Tailwind's relative order for each input class. Unknown classes map to `None`.
    pub fn class_order(&self, classes: &[&str]) -> Vec<Option<u32>> {
        let mut known = classes
            .iter()
            .enumerate()
            .filter_map(|(index, class_name)| self.sort_key(class_name).map(|key| (index, key)))
            .collect::<Vec<_>>();
        known.sort_by(|(_, left), (_, right)| compare_sort_keys(left, right));

        let mut result = vec![None; classes.len()];
        for (order, (index, _)) in known.into_iter().enumerate() {
            result[index] = Some(u32::try_from(order).unwrap_or(u32::MAX));
        }
        result
    }

    pub fn variant_order<'a>(
        &self,
        classes: impl IntoIterator<Item = &'a str>,
    ) -> FxHashMap<CompactString, u32> {
        let mut result = FxHashMap::default();
        for class_name in classes {
            for candidate in self.parse_candidate(class_name) {
                for variant in &candidate.variants {
                    let name = variant.print();
                    let root = variant_root(variant);
                    let order = self.variant_sort_order(variant).unwrap_or(0);
                    let global = u32::from(root != "[]" && self.variant_is_global(root)) << 30;
                    result.entry(CompactString::new(name)).or_insert(global | order);
                }
            }
        }
        result
    }

    fn variant_sort_order(&self, variant: &Variant) -> Option<u32> {
        match &variant.kind {
            VariantKind::Arbitrary { .. } => Some(ARBITRARY_VARIANT_ORDER),
            VariantKind::Functional { root, value: Some(VariantValue::Named(value)), .. }
                if matches!(root.as_str(), "min" | "max") =>
            {
                let breakpoint_order = self.variant_registration(value)?.order();
                if root == "min" {
                    Some(breakpoint_order)
                } else {
                    let index = breakpoint_order.checked_sub(THEME_VARIANT_ORDER)?;
                    Some(MAX_VARIANT_ORDER + THEME_VARIANT_RANGE.saturating_sub(index))
                }
            }
            VariantKind::Static { root }
            | VariantKind::Functional { root, .. }
            | VariantKind::Compound { root, .. } => {
                self.variant_registration(root).map(VariantRegistration::order)
            }
        }
    }

    pub fn conflicting_classes(&self, classes: &[&str]) -> Vec<ClassConflict> {
        let compiled =
            classes.iter().map(|class_name| self.compile_class(class_name)).collect::<Vec<_>>();
        let mut conflicts = Vec::new();
        for (left_index, left) in compiled.iter().enumerate() {
            for (right_index, right) in compiled.iter().enumerate() {
                if left_index == right_index || left.is_empty() || !same_rule_context(left, right) {
                    continue;
                }
                conflicts.push(ClassConflict {
                    class_name: CompactString::new(classes[left_index]),
                    conflicting_class_name: CompactString::new(classes[right_index]),
                    properties: right.iter().flat_map(|rule| rule.properties.clone()).collect(),
                });
            }
        }
        conflicts
    }

    fn compile_candidate(&self, candidate: &Candidate) -> Option<CompiledRule> {
        if candidate.variants.iter().any(invalid_top_level_variant) {
            return None;
        }
        let mut property_names = match &candidate.kind {
            CandidateKind::Arbitrary { property, modifier, .. } => {
                if modifier.is_some()
                    && !matches!(
                        property.as_str(),
                        "color" | "background-color" | "border-color" | "fill" | "stroke"
                    )
                {
                    return None;
                }
                vec![property.clone()]
            }
            CandidateKind::Static { root } => self.properties_for_root(root),
            CandidateKind::Functional { root, value, modifier } => {
                if !self.functional_value_is_valid(root, value.as_ref(), modifier.as_ref()) {
                    return None;
                }
                self.properties_for_functional(root, value.as_ref())
            }
        };
        if property_names.is_empty() {
            return None;
        }
        let sort_property = utility_sort_property(candidate).map(CompactString::new);
        if sort_property.is_some() {
            property_names.insert(0, CompactString::new("--tw-sort"));
        }
        let mut contexts = candidate.variants.iter().rev().map(Variant::print).collect::<Vec<_>>();
        if let Some(context) = utility_internal_context(candidate) {
            contexts.push(context.to_owned());
        }
        let context = contexts.join(":");
        Some(CompiledRule {
            context: CompactString::new(context),
            sort_property,
            properties: property_names
                .into_iter()
                .map(|css_property_name| Property {
                    css_property_name,
                    important: candidate.important,
                })
                .collect(),
        })
    }

    fn functional_value_is_valid(
        &self,
        root: &str,
        value: Option<&CandidateValue>,
        modifier: Option<&Modifier>,
    ) -> bool {
        let root = root.strip_prefix('-').unwrap_or(root);
        if self.custom_utility_properties(root).is_some() {
            return value.is_some();
        }
        let Some(value) = value else {
            return modifier.is_none() && has_default_value(root);
        };
        let fraction_modifier = matches!(
            value,
            CandidateValue::Named(value)
                if value.fraction.as_deref().is_some_and(valid_fraction)
                    && supports_fraction(root)
        );
        if !fraction_modifier
            && modifier
                .is_some_and(|modifier| !self.functional_modifier_is_valid(root, value, modifier))
        {
            return false;
        }
        if matches!(value, CandidateValue::Arbitrary(_)) {
            return true;
        }
        let CandidateValue::Named(named_value) = value else { unreachable!() };
        let value = named_value.value.as_str();

        if root == "bg" {
            return self.has_theme_color(value)
                || self
                    .theme
                    .contains_key(&CompactString::new(format!("--background-image-{value}")))
                || matches!(value, "current" | "inherit" | "transparent");
        }
        if is_color_or_width_root(root) {
            return self.has_theme_color(value)
                || matches!(value, "current" | "inherit" | "transparent")
                || value.parse::<u32>().is_ok()
                || width_theme_key_exists(&self.theme, root, value);
        }
        if matches!(root, "from" | "via" | "to") && value.ends_with('%') {
            return value[..value.len() - 1].parse::<f64>().is_ok();
        }
        if is_color_root(root) {
            return matches!(value, "current" | "inherit" | "transparent")
                || self.theme.contains_key(&CompactString::new(format!("--color-{value}")))
                || theme_key_exists(&self.theme, root, value);
        }
        if root == "text" {
            return self.theme.contains_key(&CompactString::new(format!("--text-{value}")))
                || self.theme.contains_key(&CompactString::new(format!("--color-{value}")))
                || matches!(value, "current" | "inherit" | "transparent");
        }
        if root == "font" {
            return self.theme.contains_key(&CompactString::new(format!("--font-{value}")))
                || self.theme.contains_key(&CompactString::new(format!("--font-weight-{value}")));
        }
        if is_spacing_root(root) {
            return value == "px"
                || valid_spacing_multiplier(value) && self.theme.contains_key("--spacing")
                || named_value.fraction.is_some()
                || is_builtin_spacing_value(root, value)
                || theme_key_exists(&self.theme, root, value);
        }
        if is_positive_integer_root(root) {
            return is_positive_integer(value)
                || matches!(value, "auto" | "first" | "last" | "full" | "none")
                || theme_key_exists(&self.theme, root, value);
        }
        if root == "opacity" || root.ends_with("-opacity") {
            return valid_spacing_multiplier(value)
                || self.theme.contains_key(&CompactString::new(format!("--opacity-{value}")));
        }
        if matches!(root, "grid-cols" | "grid-rows") {
            return is_strict_positive_integer(value) || theme_key_exists(&self.theme, root, value);
        }
        if is_integer_bare_value_root(root) {
            return is_positive_integer(value) || theme_key_exists(&self.theme, root, value);
        }
        if matches!(root, "blur" | "backdrop-blur") {
            return value == "none" || theme_key_exists(&self.theme, root, value);
        }
        if root.starts_with("rounded") {
            return matches!(value, "none" | "full") || theme_key_exists(&self.theme, root, value);
        }
        if matches!(root, "origin" | "transition") {
            return theme_key_exists(&self.theme, root, value)
                || is_builtin_functional_value(root, value);
        }
        theme_key_exists(&self.theme, root, value)
            || is_builtin_functional_value(root, value)
            || is_builtin_named_value(value)
    }

    fn functional_modifier_is_valid(
        &self,
        root: &str,
        value: &CandidateValue,
        modifier: &Modifier,
    ) -> bool {
        let root = root.strip_prefix('-').unwrap_or(root);
        if root == "bg-linear" {
            return matches!(value, CandidateValue::Named(_));
        }
        if matches!(root, "shadow" | "inset-shadow" | "drop-shadow" | "text-shadow") {
            return self.opacity_modifier_is_valid(modifier);
        }
        if root == "text" {
            let is_font_size = match value {
                CandidateValue::Named(value) => {
                    self.theme.contains_key(&CompactString::new(format!("--text-{}", value.value)))
                }
                CandidateValue::Arbitrary(value) => {
                    value.data_type.as_deref() == Some("length") || is_css_length(&value.value)
                }
            };
            return if is_font_size {
                match modifier {
                    Modifier::Arbitrary(_) => true,
                    Modifier::Named(value) => {
                        value == "none"
                            || self
                                .theme
                                .contains_key(&CompactString::new(format!("--leading-{value}")))
                            || valid_spacing_multiplier(value)
                                && self.theme.contains_key("--spacing")
                    }
                }
            } else {
                self.opacity_modifier_is_valid(modifier)
            };
        }
        if is_color_root(root) || root == "bg" {
            let is_color = match value {
                CandidateValue::Named(value) => {
                    self.has_theme_color(&value.value)
                        || matches!(value.value.as_str(), "current" | "inherit" | "transparent")
                }
                CandidateValue::Arbitrary(value) => value_is_arbitrary_color(value),
            };
            return is_color && self.opacity_modifier_is_valid(modifier);
        }
        false
    }

    fn opacity_modifier_is_valid(&self, modifier: &Modifier) -> bool {
        match modifier {
            Modifier::Arbitrary(_) => true,
            Modifier::Named(value) => {
                valid_spacing_multiplier(value)
                    || self.theme.contains_key(&CompactString::new(format!("--opacity-{value}")))
            }
        }
    }

    fn has_theme_color(&self, value: &str) -> bool {
        self.theme.contains_key(&CompactString::new(format!("--color-{value}")))
    }

    fn properties_for_root(&self, root: &str) -> Vec<CompactString> {
        if let Some(properties) = self.custom_utility_properties(root) {
            return properties.to_vec();
        }
        static_properties(root)
    }

    fn properties_for_functional(
        &self,
        root: &str,
        value: Option<&CandidateValue>,
    ) -> Vec<CompactString> {
        let root = root.strip_prefix('-').unwrap_or(root);
        if let Some(properties) = self.custom_utility_properties(root) {
            return properties.to_vec();
        }
        if root == "text" {
            return match value {
                Some(CandidateValue::Named(value))
                    if self
                        .theme
                        .contains_key(&CompactString::new(format!("--text-{}", value.value))) =>
                {
                    properties!["font-size", "line-height"]
                }
                Some(CandidateValue::Arbitrary(value))
                    if value.data_type.as_deref() == Some("length")
                        || is_css_length(&value.value) =>
                {
                    properties!["font-size"]
                }
                _ => properties!["color"],
            };
        }
        if root == "bg" {
            return match value {
                Some(CandidateValue::Named(value))
                    if self.theme.contains_key(&CompactString::new(format!(
                        "--background-image-{}",
                        value.value
                    ))) =>
                {
                    properties!["background-image"]
                }
                Some(CandidateValue::Arbitrary(value))
                    if matches!(value.data_type.as_deref(), Some("image" | "url"))
                        || value.value.starts_with("url(")
                        || value.value.contains("gradient(") =>
                {
                    properties!["background-image"]
                }
                _ => properties!["background-color"],
            };
        }
        if is_color_or_width_root(root) {
            let is_color = match value {
                Some(CandidateValue::Named(value)) => {
                    self.has_theme_color(&value.value)
                        || matches!(value.value.as_str(), "current" | "inherit" | "transparent")
                }
                Some(CandidateValue::Arbitrary(value)) => value_is_arbitrary_color(value),
                None => false,
            };
            return properties_for_color_or_width(root, is_color);
        }
        if root == "font" {
            return match value {
                Some(CandidateValue::Named(value))
                    if self.theme.contains_key(&CompactString::new(format!(
                        "--font-weight-{}",
                        value.value
                    ))) =>
                {
                    properties!["--tw-font-weight", "font-weight"]
                }
                Some(CandidateValue::Arbitrary(value))
                    if matches!(value.data_type.as_deref(), Some("number"))
                        || value.value.parse::<f64>().is_ok() =>
                {
                    properties!["--tw-font-weight", "font-weight"]
                }
                _ => properties!["font-family"],
            };
        }
        functional_properties(root, value)
    }

    fn sort_key(&self, class_name: &str) -> Option<SortKey> {
        let candidate = self
            .cached_candidates(class_name)
            .iter()
            .find(|candidate| self.compile_candidate(candidate).is_some())?
            .clone();
        let rules = self.compile_class(class_name);
        let rule = rules.first()?;
        let mut variants = Vec::<u64>::new();
        for variant in &candidate.variants {
            let order = self.variant_sort_order(variant)?;
            let order = usize::try_from(order).expect("variant order should fit usize");
            let limb = order / u64::BITS as usize;
            if variants.len() <= limb {
                variants.resize(limb + 1, 0);
            }
            variants[limb] |= 1_u64 << (order % u64::BITS as usize);
        }
        let sort_rank = rule.sort_property.as_ref().map(|property| property_rank(property));
        let mut property_order = match sort_rank {
            Some(rank) if rank != u16::MAX => vec![rank],
            _ => rule
                .properties
                .iter()
                .map(|property| property_rank(&property.css_property_name))
                .filter(|rank| *rank != u16::MAX)
                .collect::<Vec<_>>(),
        };
        property_order.sort_unstable();
        property_order.dedup();
        Some(SortKey {
            variants,
            property_count: rule.properties.len(),
            property_order,
            candidate: CompactString::new(class_name),
        })
    }
}

fn compare_sort_keys(left: &SortKey, right: &SortKey) -> Ordering {
    compare_variant_bits(&left.variants, &right.variants)
        .then_with(|| compare_property_order(&left.property_order, &right.property_order))
        .then_with(|| right.property_count.cmp(&left.property_count))
        .then_with(|| left.candidate.cmp(&right.candidate))
}

fn compare_variant_bits(left: &[u64], right: &[u64]) -> Ordering {
    for index in (0..left.len().max(right.len())).rev() {
        match (left.get(index).copied().unwrap_or(0), right.get(index).copied().unwrap_or(0)) {
            (left, right) if left == right => {}
            (left, right) => return left.cmp(&right),
        }
    }
    Ordering::Equal
}

fn compare_property_order(left: &[u16], right: &[u16]) -> Ordering {
    for index in 0..left.len().max(right.len()) {
        match (left.get(index), right.get(index)) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => return left.cmp(right),
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            (None, None) => break,
        }
    }
    Ordering::Equal
}

fn same_rule_context(left: &[CompiledRule], right: &[CompiledRule]) -> bool {
    left.len() == right.len()
        && left.iter().all(|left_rule| {
            right.iter().any(|right_rule| {
                left_rule.context == right_rule.context
                    && left_rule.properties.len() == right_rule.properties.len()
                    && left_rule.properties.iter().all(|left_property| {
                        right_rule.properties.iter().any(|right_property| {
                            left_property.css_property_name == right_property.css_property_name
                        })
                    })
            })
        })
}

fn invalid_top_level_variant(variant: &Variant) -> bool {
    matches!(variant.kind, VariantKind::Arbitrary { relative: true, .. })
}

fn utility_internal_context(candidate: &Candidate) -> Option<&'static str> {
    let root = match &candidate.kind {
        CandidateKind::Arbitrary { .. } => return None,
        CandidateKind::Static { root } | CandidateKind::Functional { root, .. } => {
            root.strip_prefix('-').unwrap_or(root)
        }
    };
    if root.starts_with("divide-")
        || root.starts_with("space-x")
        || root.starts_with("space-y")
        || root == "divide"
    {
        Some(":where(& > :not(:last-child))")
    } else if root == "placeholder" {
        Some("&::placeholder")
    } else {
        None
    }
}

fn utility_sort_property(candidate: &Candidate) -> Option<&'static str> {
    let (root, value) = match &candidate.kind {
        CandidateKind::Arbitrary { .. } => return None,
        CandidateKind::Static { root } => (root.strip_prefix('-').unwrap_or(root), None),
        CandidateKind::Functional { root, value, .. } => {
            (root.strip_prefix('-').unwrap_or(root), value.as_ref())
        }
    };
    match root {
        "container" => Some("--tw-container-component"),
        "size" | "size-auto" | "size-fit" | "size-full" | "size-max" | "size-min" | "size-px" => {
            Some("size")
        }
        "space-x" | "space-x-px" | "space-x-reverse" => Some("row-gap"),
        "space-y" | "space-y-px" | "space-y-reverse" => Some("column-gap"),
        "divide-x" => Some("divide-x-width"),
        "divide-y" => Some("divide-y-width"),
        "divide-solid" | "divide-dashed" | "divide-dotted" | "divide-double" | "divide-none" => {
            Some("divide-style")
        }
        "divide" => Some("divide-color"),
        "from" if !value_is_percentage(value) => Some("--tw-gradient-from"),
        "via" if !value_is_percentage(value) => Some("--tw-gradient-via"),
        "to" if !value_is_percentage(value) => Some("--tw-gradient-to"),
        "placeholder" => Some("placeholder-color"),
        _ => None,
    }
}

fn variant_root(variant: &Variant) -> &str {
    match &variant.kind {
        VariantKind::Arbitrary { .. } => "[]",
        VariantKind::Static { root }
        | VariantKind::Functional { root, .. }
        | VariantKind::Compound { root, .. } => root,
    }
}

fn theme_key_exists(
    theme: &rustc_hash::FxHashMap<CompactString, CompactString>,
    root: &str,
    value: &str,
) -> bool {
    let namespaces: &[&str] = match root {
        "bg" | "text" | "fill" | "stroke" | "accent" | "caret" | "placeholder" => &["color"],
        root if root.starts_with("rounded") => &["radius"],
        "font" => &["font", "font-weight"],
        "leading" => &["leading"],
        "tracking" => &["tracking"],
        "shadow" | "inset-shadow" => &["shadow"],
        "drop-shadow" => &["drop-shadow"],
        "blur" => &["blur"],
        "backdrop-blur" => &["backdrop-blur", "blur"],
        "w" => &["width", "spacing", "container"],
        "min-w" => &["min-width", "spacing", "container"],
        "max-w" => &["max-width", "spacing", "container"],
        "h" => &["height", "spacing"],
        "min-h" => &["min-height", "height", "spacing"],
        "max-h" => &["max-height", "height", "spacing"],
        "auto-cols" => &["grid-auto-columns"],
        "auto-rows" => &["grid-auto-rows"],
        other => return theme.contains_key(&CompactString::new(format!("--{other}-{value}"))),
    };
    namespaces
        .iter()
        .any(|namespace| theme.contains_key(&CompactString::new(format!("--{namespace}-{value}"))))
}

fn is_builtin_spacing_value(root: &str, value: &str) -> bool {
    match root {
        "inset" | "inset-x" | "inset-y" | "inset-s" | "inset-e" | "inset-bs" | "inset-be"
        | "top" | "right" | "bottom" | "left" | "start" | "end" => {
            matches!(value, "auto" | "full")
        }
        "m" | "mx" | "my" | "ms" | "me" | "mbs" | "mbe" | "mt" | "mr" | "mb" | "ml" => {
            value == "auto"
        }
        "size" => matches!(
            value,
            "auto" | "full" | "svw" | "lvw" | "dvw" | "svh" | "lvh" | "dvh" | "min" | "max" | "fit"
        ),
        "w" | "min-w" | "max-w" | "h" | "min-h" | "max-h" => matches!(
            value,
            "auto"
                | "full"
                | "svw"
                | "lvw"
                | "dvw"
                | "svh"
                | "lvh"
                | "dvh"
                | "min"
                | "max"
                | "fit"
                | "screen"
                | "lh"
                | "none"
        ),
        "inline" | "min-inline" | "max-inline" => {
            matches!(
                value,
                "auto" | "full" | "svw" | "lvw" | "dvw" | "min" | "max" | "fit" | "screen"
            )
        }
        "block" | "min-block" | "max-block" => {
            matches!(
                value,
                "auto" | "full" | "svh" | "lvh" | "dvh" | "min" | "max" | "fit" | "screen" | "lh"
            )
        }
        "basis" | "translate" | "translate-x" | "translate-y" => value == "full",
        _ => false,
    }
}

fn is_builtin_functional_value(root: &str, value: &str) -> bool {
    match root {
        "auto-cols" | "auto-rows" => matches!(value, "auto" | "min" | "max" | "fr"),
        "list" => matches!(value, "none" | "disc" | "decimal"),
        "origin" => matches!(
            value,
            "center"
                | "top"
                | "top-right"
                | "right"
                | "bottom-right"
                | "bottom"
                | "bottom-left"
                | "left"
                | "top-left"
        ),
        "transition" => {
            matches!(value, "none" | "all" | "colors" | "opacity" | "shadow" | "transform")
        }
        "bg-linear" => {
            matches!(
                value,
                "to-t" | "to-tr" | "to-r" | "to-br" | "to-b" | "to-bl" | "to-l" | "to-tl"
            ) || value.parse::<u16>().is_ok()
        }
        _ => false,
    }
}

fn valid_spacing_multiplier(value: &str) -> bool {
    value.parse::<f64>().is_ok_and(|number| {
        number >= 0.0
            && number.is_finite()
            && (number * 4.0).fract() == 0.0
            && format_number(number) == value
    })
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { value.to_string() }
}

fn is_builtin_named_value(value: &str) -> bool {
    value.parse::<f64>().is_ok()
        || matches!(
            value,
            "auto"
                | "none"
                | "full"
                | "min"
                | "max"
                | "fit"
                | "px"
                | "screen"
                | "inherit"
                | "initial"
                | "current"
                | "transparent"
                | "normal"
                | "DEFAULT"
        )
}

fn is_positive_integer_root(root: &str) -> bool {
    matches!(
        root,
        "z" | "order"
            | "col"
            | "col-span"
            | "col-start"
            | "col-end"
            | "row"
            | "row-span"
            | "row-start"
            | "row-end"
            | "line-clamp"
            | "tab"
    )
}

fn is_positive_integer(value: &str) -> bool {
    value.parse::<u64>().is_ok_and(|number| number.to_string() == value)
}

fn is_strict_positive_integer(value: &str) -> bool {
    value.parse::<u64>().is_ok_and(|number| number > 0 && number.to_string() == value)
}

fn is_integer_bare_value_root(root: &str) -> bool {
    matches!(
        root,
        "brightness"
            | "backdrop-brightness"
            | "contrast"
            | "backdrop-contrast"
            | "grayscale"
            | "backdrop-grayscale"
            | "hue-rotate"
            | "backdrop-hue-rotate"
            | "invert"
            | "backdrop-invert"
            | "saturate"
            | "backdrop-saturate"
            | "sepia"
            | "backdrop-sepia"
            | "rotate"
            | "rotate-x"
            | "rotate-y"
            | "rotate-z"
            | "scale"
            | "scale-x"
            | "scale-y"
            | "scale-z"
            | "skew"
            | "skew-x"
            | "skew-y"
            | "grow"
            | "shrink"
            | "delay"
            | "duration"
    )
}

fn supports_fraction(root: &str) -> bool {
    matches!(
        root.strip_prefix('-').unwrap_or(root),
        "inset"
            | "inset-x"
            | "inset-y"
            | "inset-s"
            | "inset-e"
            | "inset-bs"
            | "inset-be"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "start"
            | "end"
            | "size"
            | "w"
            | "min-w"
            | "max-w"
            | "h"
            | "min-h"
            | "max-h"
            | "basis"
            | "translate"
            | "translate-x"
            | "translate-y"
            | "flex"
            | "aspect"
    )
}

fn valid_fraction(value: &str) -> bool {
    value
        .split_once('/')
        .is_some_and(|(left, right)| is_positive_integer(left) && is_positive_integer(right))
}

fn has_default_value(root: &str) -> bool {
    matches!(
        root,
        "border"
            | "border-x"
            | "border-y"
            | "border-s"
            | "border-e"
            | "border-t"
            | "border-r"
            | "border-b"
            | "border-l"
            | "rounded"
            | "shadow"
            | "inset-shadow"
            | "ring"
            | "outline"
            | "blur"
            | "brightness"
            | "contrast"
            | "grayscale"
            | "hue-rotate"
            | "invert"
            | "saturate"
            | "sepia"
            | "grow"
            | "shrink"
            | "transition"
            | "translate"
            | "rotate"
            | "scale"
    )
}

fn is_color_root(root: &str) -> bool {
    matches!(
        root,
        "accent"
            | "bg"
            | "border"
            | "border-x"
            | "border-y"
            | "border-s"
            | "border-e"
            | "border-t"
            | "border-r"
            | "border-b"
            | "border-l"
            | "caret"
            | "decoration"
            | "divide"
            | "fill"
            | "from"
            | "placeholder"
            | "ring"
            | "ring-offset"
            | "stroke"
            | "to"
            | "via"
    )
}

fn is_color_or_width_root(root: &str) -> bool {
    root == "border"
        || root.starts_with("border-")
        || matches!(
            root,
            "divide"
                | "divide-x"
                | "divide-y"
                | "ring"
                | "ring-offset"
                | "outline"
                | "decoration"
                | "stroke"
        )
}

fn width_theme_key_exists(
    theme: &rustc_hash::FxHashMap<CompactString, CompactString>,
    root: &str,
    value: &str,
) -> bool {
    let namespace = match root {
        root if root.starts_with("border") => "border-width",
        root if root.starts_with("divide") => "divide-width",
        "ring" => "ring-width",
        "ring-offset" => "ring-offset-width",
        "outline" => "outline-width",
        "decoration" => "text-decoration-thickness",
        "stroke" => "stroke-width",
        _ => return false,
    };
    theme.contains_key(&CompactString::new(format!("--{namespace}-{value}")))
}

fn properties_for_color_or_width(root: &str, is_color: bool) -> Vec<CompactString> {
    match (root, is_color) {
        ("border", true) => properties!["border-color"],
        ("border-x", true) => properties!["border-inline-color"],
        ("border-y", true) => properties!["border-block-color"],
        ("border-s", true) => properties!["border-inline-start-color"],
        ("border-e", true) => properties!["border-inline-end-color"],
        ("border-bs", true) => properties!["border-block-start-color"],
        ("border-be", true) => properties!["border-block-end-color"],
        ("border-t", true) => properties!["border-top-color"],
        ("border-r", true) => properties!["border-right-color"],
        ("border-b", true) => properties!["border-bottom-color"],
        ("border-l", true) => properties!["border-left-color"],
        ("border", false) => properties!["border-style", "border-width"],
        ("divide", false) => properties!["border-width"],
        ("border-x", false) => properties!["border-inline-style", "border-inline-width"],
        ("border-y", false) => properties!["border-block-style", "border-block-width"],
        ("divide-x", false) => properties![
            "--tw-divide-x-reverse",
            "border-inline-style",
            "border-inline-start-width",
            "border-inline-end-width"
        ],
        ("divide-y", false) => properties![
            "--tw-divide-y-reverse",
            "border-bottom-style",
            "border-top-style",
            "border-top-width",
            "border-bottom-width"
        ],
        ("border-s", false) => {
            properties!["border-inline-start-style", "border-inline-start-width"]
        }
        ("border-e", false) => {
            properties!["border-inline-end-style", "border-inline-end-width"]
        }
        ("border-bs", false) => {
            properties!["border-block-start-style", "border-block-start-width"]
        }
        ("border-be", false) => {
            properties!["border-block-end-style", "border-block-end-width"]
        }
        ("border-t", false) => properties!["border-top-style", "border-top-width"],
        ("border-r", false) => properties!["border-right-style", "border-right-width"],
        ("border-b", false) => properties!["border-bottom-style", "border-bottom-width"],
        ("border-l", false) => properties!["border-left-style", "border-left-width"],
        (root, true) if root.starts_with("divide") => properties!["border-color"],
        ("ring", true) => properties!["--tw-ring-color"],
        ("ring", false) => properties!["--tw-ring-shadow", "box-shadow"],
        ("ring-offset", true) => properties!["--tw-ring-offset-color"],
        ("ring-offset", false) => {
            properties!["--tw-ring-offset-width", "--tw-ring-offset-shadow"]
        }
        ("outline", true) => properties!["outline-color"],
        ("outline", false) => properties!["outline-style", "outline-width"],
        ("decoration", true) => properties!["text-decoration-color"],
        ("decoration", false) => properties!["text-decoration-thickness"],
        ("stroke", true) => properties!["stroke"],
        ("stroke", false) => properties!["stroke-width"],
        _ => Vec::new(),
    }
}

fn is_spacing_root(root: &str) -> bool {
    matches!(
        root,
        "inset"
            | "inset-x"
            | "inset-y"
            | "inset-s"
            | "inset-e"
            | "inset-bs"
            | "inset-be"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "start"
            | "end"
            | "m"
            | "mx"
            | "my"
            | "ms"
            | "me"
            | "mbs"
            | "mbe"
            | "mt"
            | "mr"
            | "mb"
            | "ml"
            | "size"
            | "w"
            | "min-w"
            | "max-w"
            | "h"
            | "min-h"
            | "max-h"
            | "inline"
            | "min-inline"
            | "max-inline"
            | "block"
            | "min-block"
            | "max-block"
            | "basis"
            | "gap"
            | "gap-x"
            | "gap-y"
            | "space-x"
            | "space-y"
            | "scroll-m"
            | "scroll-mx"
            | "scroll-my"
            | "scroll-ms"
            | "scroll-me"
            | "scroll-mbs"
            | "scroll-mbe"
            | "scroll-mt"
            | "scroll-mr"
            | "scroll-mb"
            | "scroll-ml"
            | "scroll-p"
            | "scroll-px"
            | "scroll-py"
            | "scroll-ps"
            | "scroll-pe"
            | "scroll-pbs"
            | "scroll-pbe"
            | "scroll-pt"
            | "scroll-pr"
            | "scroll-pb"
            | "scroll-pl"
            | "p"
            | "px"
            | "py"
            | "ps"
            | "pe"
            | "pbs"
            | "pbe"
            | "pt"
            | "pr"
            | "pb"
            | "pl"
            | "indent"
            | "leading"
            | "translate"
            | "translate-x"
            | "translate-y"
            | "translate-z"
    )
}

fn static_properties(root: &str) -> Vec<CompactString> {
    let root = root.strip_prefix('-').unwrap_or(root);
    if root == "sr-only" {
        return properties![
            "position",
            "width",
            "height",
            "padding",
            "margin",
            "overflow",
            "clip-path",
            "white-space",
            "border-width"
        ];
    }
    if root == "not-sr-only" {
        return properties![
            "position",
            "width",
            "height",
            "padding",
            "margin",
            "overflow",
            "clip-path",
            "white-space"
        ];
    }
    if root == "container" {
        return properties!["width", "max-width"];
    }
    if matches!(
        root,
        "block"
            | "inline"
            | "inline-block"
            | "flex"
            | "inline-flex"
            | "grid"
            | "inline-grid"
            | "hidden"
            | "contents"
            | "flow-root"
            | "list-item"
            | "table"
            | "inline-table"
            | "table-caption"
            | "table-cell"
            | "table-column"
            | "table-column-group"
            | "table-footer-group"
            | "table-header-group"
            | "table-row-group"
            | "table-row"
    ) {
        return properties!["display"];
    }
    if matches!(root, "static" | "fixed" | "absolute" | "relative" | "sticky") {
        return properties!["position"];
    }
    for (prefix, property) in [
        ("inset-x-", "inset-inline"),
        ("inset-y-", "inset-block"),
        ("inset-s-", "inset-inline-start"),
        ("inset-e-", "inset-inline-end"),
        ("inset-bs-", "inset-block-start"),
        ("inset-be-", "inset-block-end"),
        ("inset-", "inset"),
        ("top-", "top"),
        ("right-", "right"),
        ("bottom-", "bottom"),
        ("left-", "left"),
        ("start-", "inset-inline-start"),
        ("end-", "inset-inline-end"),
        ("scroll-mx-", "scroll-margin-inline"),
        ("scroll-my-", "scroll-margin-block"),
        ("scroll-ms-", "scroll-margin-inline-start"),
        ("scroll-me-", "scroll-margin-inline-end"),
        ("scroll-mbs-", "scroll-margin-block-start"),
        ("scroll-mbe-", "scroll-margin-block-end"),
        ("scroll-mt-", "scroll-margin-top"),
        ("scroll-mr-", "scroll-margin-right"),
        ("scroll-mb-", "scroll-margin-bottom"),
        ("scroll-ml-", "scroll-margin-left"),
        ("scroll-m-", "scroll-margin"),
        ("mx-", "margin-inline"),
        ("my-", "margin-block"),
        ("ms-", "margin-inline-start"),
        ("me-", "margin-inline-end"),
        ("mbs-", "margin-block-start"),
        ("mbe-", "margin-block-end"),
        ("mt-", "margin-top"),
        ("mr-", "margin-right"),
        ("mb-", "margin-bottom"),
        ("ml-", "margin-left"),
        ("m-", "margin"),
        ("px-", "padding-inline"),
        ("py-", "padding-block"),
        ("ps-", "padding-inline-start"),
        ("pe-", "padding-inline-end"),
        ("pbs-", "padding-block-start"),
        ("pbe-", "padding-block-end"),
        ("pt-", "padding-top"),
        ("pr-", "padding-right"),
        ("pb-", "padding-bottom"),
        ("pl-", "padding-left"),
        ("p-", "padding"),
    ] {
        if root.starts_with(prefix) {
            return properties![property];
        }
    }
    for (prefix, property) in [
        ("block-", "block-size"),
        ("min-block-", "min-block-size"),
        ("max-block-", "max-block-size"),
        ("inline-", "inline-size"),
        ("min-inline-", "min-inline-size"),
        ("max-inline-", "max-inline-size"),
        ("h-", "height"),
        ("min-h-", "min-height"),
        ("max-h-", "max-height"),
        ("w-", "width"),
        ("min-w-", "min-width"),
        ("max-w-", "max-width"),
        ("basis-", "flex-basis"),
        ("gap-x-", "column-gap"),
        ("gap-y-", "row-gap"),
        ("gap-", "gap"),
        ("indent-", "text-indent"),
        ("leading-", "line-height"),
    ] {
        if root.starts_with(prefix) {
            return properties![property];
        }
    }
    if root.starts_with("size-") {
        return properties!["width", "height"];
    }
    if root.starts_with("border-spacing-x-") {
        return properties!["--tw-border-spacing-x", "border-spacing"];
    }
    if root.starts_with("border-spacing-y-") {
        return properties!["--tw-border-spacing-y", "border-spacing"];
    }
    if root.starts_with("border-spacing-") {
        return properties!["--tw-border-spacing-x", "--tw-border-spacing-y", "border-spacing"];
    }
    if root == "snap-mandatory" || root == "snap-proximity" {
        return properties!["--tw-scroll-snap-strictness"];
    }
    if root.starts_with("touch-pan-") {
        let axis = if matches!(root, "touch-pan-x" | "touch-pan-left" | "touch-pan-right") {
            "--tw-pan-x"
        } else {
            "--tw-pan-y"
        };
        return properties![axis, "touch-action"];
    }
    if root == "touch-pinch-zoom" {
        return properties!["--tw-pinch-zoom", "touch-action"];
    }
    if matches!(root, "touch-auto" | "touch-manipulation" | "touch-none") {
        return properties!["touch-action"];
    }
    if root.starts_with("select-") {
        return properties!["-webkit-user-select", "user-select"];
    }
    if matches!(root, "flex-auto" | "flex-initial" | "flex-none") {
        return properties!["flex"];
    }
    if matches!(root, "flex-row" | "flex-row-reverse" | "flex-col" | "flex-col-reverse") {
        return properties!["flex-direction"];
    }
    if matches!(root, "flex-wrap" | "flex-wrap-reverse" | "flex-nowrap") {
        return properties!["flex-wrap"];
    }
    if root.starts_with("grid-flow-") {
        return properties!["grid-auto-flow"];
    }
    if root.starts_with("font-stretch-") {
        return properties!["font-stretch"];
    }
    if matches!(root, "isolate" | "isolation-auto") {
        return properties!["isolation"];
    }
    if root.starts_with("backface-") {
        return properties!["backface-visibility"];
    }
    if root.starts_with("forced-color-adjust-") {
        return properties!["forced-color-adjust"];
    }
    if root.starts_with("scheme-") {
        return properties!["color-scheme"];
    }
    if matches!(root, "scrollbar-auto" | "scrollbar-none" | "scrollbar-thin") {
        return properties!["scrollbar-width"];
    }
    if root.starts_with("scrollbar-gutter-") {
        return properties!["scrollbar-gutter"];
    }
    if matches!(root, "snap-align-none" | "snap-center" | "snap-end" | "snap-start") {
        return properties!["scroll-snap-align"];
    }
    if matches!(root, "snap-always" | "snap-normal") {
        return properties!["scroll-snap-stop"];
    }
    if matches!(root, "snap-both" | "snap-x" | "snap-y" | "snap-none") {
        return properties!["scroll-snap-type"];
    }
    for (prefix, property) in [
        ("pointer-events-", "pointer-events"),
        ("isolation-", "isolation"),
        ("float-", "float"),
        ("clear-", "clear"),
        ("box-decoration-", "box-decoration-break"),
        ("box-", "box-sizing"),
        ("field-sizing-", "field-sizing"),
        ("table-", "table-layout"),
        ("caption-", "caption-side"),
        ("border-collapse", "border-collapse"),
        ("border-separate", "border-collapse"),
        ("overflow-x-", "overflow-x"),
        ("overflow-y-", "overflow-y"),
        ("overflow-", "overflow"),
        ("overscroll-x-", "overscroll-behavior-x"),
        ("overscroll-y-", "overscroll-behavior-y"),
        ("overscroll-", "overscroll-behavior"),
        ("columns-", "columns"),
        ("break-after-", "break-after"),
        ("break-before-", "break-before"),
        ("break-inside-", "break-inside"),
        ("list-inside", "list-style-position"),
        ("list-outside", "list-style-position"),
        ("appearance-", "appearance"),
        ("select-", "user-select"),
        ("resize-", "resize"),
        ("scroll-auto", "scroll-behavior"),
        ("scroll-smooth", "scroll-behavior"),
        ("touch-", "touch-action"),
        ("whitespace-", "white-space"),
        ("hyphens-", "hyphens"),
        ("align-", "vertical-align"),
        ("bg-fixed", "background-attachment"),
        ("bg-local", "background-attachment"),
        ("bg-scroll", "background-attachment"),
        ("bg-clip-", "background-clip"),
        ("bg-origin-", "background-origin"),
        ("bg-repeat", "background-repeat"),
        ("bg-blend-", "background-blend-mode"),
        ("mix-blend-", "mix-blend-mode"),
        ("uppercase", "text-transform"),
        ("lowercase", "text-transform"),
        ("capitalize", "text-transform"),
        ("normal-case", "text-transform"),
        ("italic", "font-style"),
        ("not-italic", "font-style"),
        ("antialiased", "-webkit-font-smoothing"),
        ("subpixel-antialiased", "-webkit-font-smoothing"),
        ("cursor-", "cursor"),
        ("caret-", "caret-color"),
        ("accent-", "accent-color"),
        ("will-change-", "will-change"),
    ] {
        if root.starts_with(prefix) {
            return properties![property];
        }
    }
    if matches!(root, "visible" | "invisible" | "collapse") {
        return properties!["visibility"];
    }
    if matches!(root, "contain-content" | "contain-none" | "contain-strict") {
        return properties!["contain"];
    }
    for (utility, variable) in [
        ("contain-inline-size", "--tw-contain-size"),
        ("contain-size", "--tw-contain-size"),
        ("contain-layout", "--tw-contain-layout"),
        ("contain-paint", "--tw-contain-paint"),
        ("contain-style", "--tw-contain-style"),
    ] {
        if root == utility {
            return properties![variable, "contain"];
        }
    }
    if matches!(root, "diagonal-fractions" | "stacked-fractions") {
        return properties!["--tw-numeric-fraction", "font-variant-numeric"];
    }
    if matches!(root, "lining-nums" | "oldstyle-nums") {
        return properties!["--tw-numeric-figure", "font-variant-numeric"];
    }
    if matches!(root, "proportional-nums" | "tabular-nums") {
        return properties!["--tw-numeric-spacing", "font-variant-numeric"];
    }
    if root == "ordinal" {
        return properties!["--tw-ordinal", "font-variant-numeric"];
    }
    if root == "slashed-zero" {
        return properties!["--tw-slashed-zero", "font-variant-numeric"];
    }
    if root == "normal-nums" {
        return properties!["font-variant-numeric"];
    }
    if root == "divide-x-reverse" {
        return properties!["--tw-divide-x-reverse"];
    }
    if root == "divide-y-reverse" {
        return properties!["--tw-divide-y-reverse"];
    }
    if root == "space-x-reverse" {
        return properties!["--tw-space-x-reverse"];
    }
    if root == "space-y-reverse" {
        return properties!["--tw-space-y-reverse"];
    }
    if root == "space-x-px" {
        return properties!["--tw-space-x-reverse", "margin-inline-start", "margin-inline-end"];
    }
    if root == "space-y-px" {
        return properties!["--tw-space-y-reverse", "margin-block-start", "margin-block-end"];
    }
    if root == "duration-initial" {
        return properties!["--tw-duration"];
    }
    if root == "ring-inset" {
        return properties!["--tw-ring-inset"];
    }
    if root == "shadow-initial" {
        return properties!["--tw-shadow-color"];
    }
    if root == "inset-shadow-initial" {
        return properties!["--tw-inset-shadow-color"];
    }
    if root == "text-shadow-initial" {
        return properties!["--tw-text-shadow-color"];
    }
    if root == "via-none" {
        return properties!["--tw-gradient-via-stops"];
    }
    if root == "drop-shadow-none" {
        return properties!["--tw-drop-shadow", "filter"];
    }
    if matches!(root, "fill-none") {
        return properties!["fill"];
    }
    if matches!(root, "stroke-none") {
        return properties!["stroke"];
    }
    if root == "transform-cpu" || root == "transform-gpu" || root == "transform-none" {
        return properties!["transform"];
    }
    if matches!(root, "transform-3d" | "transform-flat") {
        return properties!["transform-style"];
    }
    if matches!(
        root,
        "transform-border"
            | "transform-content"
            | "transform-fill"
            | "transform-stroke"
            | "transform-view"
    ) {
        return properties!["transform-box"];
    }
    if root == "translate-3d" || root == "translate-none" {
        return properties!["translate"];
    }
    if root == "rotate-none" {
        return properties!["rotate"];
    }
    if root == "scale-3d" || root == "scale-none" {
        return properties!["scale"];
    }
    if root.starts_with("transition-")
        && matches!(root, "transition-discrete" | "transition-normal")
    {
        return properties!["transition-behavior"];
    }
    if root == "content-none" {
        return properties!["--tw-content", "content"];
    }
    for (prefix, property) in [
        ("place-content-", "place-content"),
        ("place-items-", "place-items"),
        ("content-", "align-content"),
        ("items-", "align-items"),
        ("justify-items-", "justify-items"),
        ("justify-", "justify-content"),
        ("place-self-", "place-self"),
        ("self-", "align-self"),
        ("justify-self-", "justify-self"),
    ] {
        if root.starts_with(prefix) {
            return properties![property];
        }
    }
    if matches!(
        root,
        "border"
            | "border-x"
            | "border-y"
            | "border-s"
            | "border-e"
            | "border-bs"
            | "border-be"
            | "border-t"
            | "border-r"
            | "border-b"
            | "border-l"
    ) {
        return properties_for_color_or_width(root, false);
    }
    if root == "outline" {
        return properties!["outline-style", "outline-width"];
    }
    if root == "ring" {
        return properties!["--tw-ring-shadow", "box-shadow"];
    }
    if matches!(
        root,
        "object-contain" | "object-cover" | "object-fill" | "object-none" | "object-scale-down"
    ) {
        return properties!["object-fit"];
    }
    if root.starts_with("object-") {
        return properties!["object-position"];
    }
    if matches!(root, "bg-auto" | "bg-cover" | "bg-contain") {
        return properties!["background-size"];
    }
    if matches!(
        root,
        "bg-bottom"
            | "bg-bottom-left"
            | "bg-bottom-right"
            | "bg-center"
            | "bg-left"
            | "bg-left-bottom"
            | "bg-left-top"
            | "bg-right"
            | "bg-right-bottom"
            | "bg-right-top"
            | "bg-top"
            | "bg-top-left"
            | "bg-top-right"
    ) {
        return properties!["background-position"];
    }
    if root == "bg-none" {
        return properties!["background-image"];
    }
    if root.starts_with("bg-gradient-to-") {
        return properties!["--tw-gradient-position", "background-image"];
    }
    if matches!(root, "mask-add" | "mask-subtract" | "mask-intersect" | "mask-exclude") {
        return properties!["mask-composite"];
    }
    if matches!(root, "mask-alpha" | "mask-luminance" | "mask-match") {
        return properties!["mask-mode"];
    }
    if matches!(root, "mask-auto" | "mask-contain" | "mask-cover") {
        return properties!["mask-size"];
    }
    if matches!(
        root,
        "mask-bottom"
            | "mask-bottom-left"
            | "mask-bottom-right"
            | "mask-center"
            | "mask-left"
            | "mask-right"
            | "mask-top"
            | "mask-top-left"
            | "mask-top-right"
    ) {
        return properties!["mask-position"];
    }
    if matches!(root, "mask-circle" | "mask-ellipse") {
        return properties!["--tw-mask-radial-shape"];
    }
    if root.starts_with("mask-clip-") || root == "mask-no-clip" {
        return properties!["mask-clip"];
    }
    if root == "mask-none" {
        return properties!["mask-image"];
    }
    if root.starts_with("mask-origin-") {
        return properties!["mask-origin"];
    }
    if matches!(
        root,
        "mask-radial-at-bottom"
            | "mask-radial-at-bottom-left"
            | "mask-radial-at-bottom-right"
            | "mask-radial-at-center"
            | "mask-radial-at-left"
            | "mask-radial-at-right"
            | "mask-radial-at-top"
            | "mask-radial-at-top-left"
            | "mask-radial-at-top-right"
    ) {
        return properties!["--tw-mask-radial-position"];
    }
    if matches!(
        root,
        "mask-radial-closest-corner"
            | "mask-radial-closest-side"
            | "mask-radial-farthest-corner"
            | "mask-radial-farthest-side"
    ) {
        return properties!["--tw-mask-radial-size"];
    }
    if matches!(root, "mask-type-alpha" | "mask-type-luminance") {
        return properties!["mask-type"];
    }
    if root == "mask-no-repeat" || root.starts_with("mask-repeat") {
        return properties!["mask-repeat"];
    }
    if matches!(root, "truncate") {
        return properties!["overflow", "text-overflow", "white-space"];
    }
    if matches!(root, "text-ellipsis" | "text-clip") {
        return properties!["text-overflow"];
    }
    if matches!(root, "text-wrap" | "text-nowrap" | "text-balance" | "text-pretty") {
        return properties!["text-wrap"];
    }
    if matches!(root, "wrap-anywhere" | "wrap-break-word" | "wrap-normal") {
        return properties!["overflow-wrap"];
    }
    if matches!(root, "break-normal" | "break-words" | "break-all" | "break-keep") {
        return properties!["overflow-wrap", "word-break"];
    }
    if matches!(
        root,
        "border-solid"
            | "border-dashed"
            | "border-dotted"
            | "border-double"
            | "border-hidden"
            | "border-none"
    ) {
        return properties!["border-style"];
    }
    if matches!(
        root,
        "divide-solid" | "divide-dashed" | "divide-dotted" | "divide-double" | "divide-none"
    ) {
        return properties!["border-style"];
    }
    if matches!(
        root,
        "decoration-solid"
            | "decoration-double"
            | "decoration-dotted"
            | "decoration-dashed"
            | "decoration-wavy"
    ) {
        return properties!["text-decoration-style"];
    }
    if matches!(root, "decoration-auto" | "decoration-from-font") {
        return properties!["text-decoration-thickness"];
    }
    if matches!(root, "decoration-clone" | "decoration-slice") {
        return properties!["box-decoration-break"];
    }
    if matches!(
        root,
        "outline-solid"
            | "outline-dashed"
            | "outline-dotted"
            | "outline-double"
            | "outline-none"
            | "outline-hidden"
    ) {
        return properties!["outline-style"];
    }
    if root.starts_with("text-")
        && matches!(
            root,
            "text-left" | "text-center" | "text-right" | "text-justify" | "text-start" | "text-end"
        )
    {
        return properties!["text-align"];
    }
    if matches!(root, "underline" | "overline" | "line-through" | "no-underline") {
        return properties!["text-decoration-line"];
    }
    if root == "resize" {
        return properties!["resize"];
    }
    if root == "order-none" {
        return properties!["order"];
    }
    if root.starts_with("m-") || root == "m-auto" {
        return properties!["margin"];
    }
    if root.starts_with("mx-") {
        return properties!["margin-inline"];
    }
    if root.starts_with("my-") {
        return properties!["margin-block"];
    }
    if root.starts_with("mt-") {
        return properties!["margin-top"];
    }
    if root.starts_with("mr-") {
        return properties!["margin-right"];
    }
    if root.starts_with("mb-") {
        return properties!["margin-bottom"];
    }
    if root.starts_with("ml-") {
        return properties!["margin-left"];
    }
    if root.starts_with("p-") {
        return properties!["padding"];
    }
    if root.starts_with("px-") {
        return properties!["padding-inline"];
    }
    if root.starts_with("py-") {
        return properties!["padding-block"];
    }
    if root.starts_with("pt-") {
        return properties!["padding-top"];
    }
    if root.starts_with("pr-") {
        return properties!["padding-right"];
    }
    if root.starts_with("pb-") {
        return properties!["padding-bottom"];
    }
    if root.starts_with("pl-") {
        return properties!["padding-left"];
    }
    if root.starts_with("w-") {
        return properties!["width"];
    }
    if root.starts_with("h-") {
        return properties!["height"];
    }
    if root.starts_with("min-w-") {
        return properties!["min-width"];
    }
    if root.starts_with("max-w-") {
        return properties!["max-width"];
    }
    if root.starts_with("min-h-") {
        return properties!["min-height"];
    }
    if root.starts_with("max-h-") {
        return properties!["max-height"];
    }
    if root.starts_with("size-") {
        return properties!["width", "height"];
    }
    if root.starts_with("bg-") {
        return properties!["background-color"];
    }
    if root.starts_with("border-") {
        return properties!["border-style"];
    }
    if root.starts_with("rounded-") {
        return properties!["border-radius"];
    }
    Vec::new()
}

fn functional_properties(root: &str, value: Option<&CandidateValue>) -> Vec<CompactString> {
    match root {
        "inset" => properties!["inset"],
        "inset-x" => properties!["inset-inline"],
        "inset-y" => properties!["inset-block"],
        "inset-s" | "start" => properties!["inset-inline-start"],
        "inset-e" | "end" => properties!["inset-inline-end"],
        "inset-bs" => properties!["inset-block-start"],
        "inset-be" => properties!["inset-block-end"],
        "top" | "right" | "bottom" | "left" => properties![root],
        "m" => properties!["margin"],
        "mx" => properties!["margin-inline"],
        "my" => properties!["margin-block"],
        "ms" => properties!["margin-inline-start"],
        "me" => properties!["margin-inline-end"],
        "mbs" => properties!["margin-block-start"],
        "mbe" => properties!["margin-block-end"],
        "mt" => properties!["margin-top"],
        "mr" => properties!["margin-right"],
        "mb" => properties!["margin-bottom"],
        "ml" => properties!["margin-left"],
        "p" => properties!["padding"],
        "px" => properties!["padding-inline"],
        "py" => properties!["padding-block"],
        "ps" => properties!["padding-inline-start"],
        "pe" => properties!["padding-inline-end"],
        "pbs" => properties!["padding-block-start"],
        "pbe" => properties!["padding-block-end"],
        "pt" => properties!["padding-top"],
        "pr" => properties!["padding-right"],
        "pb" => properties!["padding-bottom"],
        "pl" => properties!["padding-left"],
        "size" => properties!["width", "height"],
        "w" => properties!["width"],
        "h" => properties!["height"],
        "inline" => properties!["inline-size"],
        "block" => properties!["block-size"],
        "min-w" => properties!["min-width"],
        "max-w" => properties!["max-width"],
        "min-h" => properties!["min-height"],
        "max-h" => properties!["max-height"],
        "min-inline" => properties!["min-inline-size"],
        "max-inline" => properties!["max-inline-size"],
        "min-block" => properties!["min-block-size"],
        "max-block" => properties!["max-block-size"],
        "scroll-m" => properties!["scroll-margin"],
        "scroll-mx" => properties!["scroll-margin-inline"],
        "scroll-my" => properties!["scroll-margin-block"],
        "scroll-ms" => properties!["scroll-margin-inline-start"],
        "scroll-me" => properties!["scroll-margin-inline-end"],
        "scroll-mbs" => properties!["scroll-margin-block-start"],
        "scroll-mbe" => properties!["scroll-margin-block-end"],
        "scroll-mt" => properties!["scroll-margin-top"],
        "scroll-mr" => properties!["scroll-margin-right"],
        "scroll-mb" => properties!["scroll-margin-bottom"],
        "scroll-ml" => properties!["scroll-margin-left"],
        "scroll-p" => properties!["scroll-padding"],
        "scroll-px" => properties!["scroll-padding-inline"],
        "scroll-py" => properties!["scroll-padding-block"],
        "scroll-ps" => properties!["scroll-padding-inline-start"],
        "scroll-pe" => properties!["scroll-padding-inline-end"],
        "scroll-pbs" => properties!["scroll-padding-block-start"],
        "scroll-pbe" => properties!["scroll-padding-block-end"],
        "scroll-pt" => properties!["scroll-padding-top"],
        "scroll-pr" => properties!["scroll-padding-right"],
        "scroll-pb" => properties!["scroll-padding-bottom"],
        "scroll-pl" => properties!["scroll-padding-left"],
        "gap" => properties!["gap"],
        "gap-x" => properties!["column-gap"],
        "gap-y" => properties!["row-gap"],
        "col" | "col-span" => properties!["grid-column"],
        "col-start" => properties!["grid-column-start"],
        "col-end" => properties!["grid-column-end"],
        "row" | "row-span" => properties!["grid-row"],
        "row-start" => properties!["grid-row-start"],
        "row-end" => properties!["grid-row-end"],
        "auto-cols" => properties!["grid-auto-columns"],
        "auto-rows" => properties!["grid-auto-rows"],
        "grid-cols" => properties!["grid-template-columns"],
        "grid-rows" => properties!["grid-template-rows"],
        "place-content" => properties!["place-content"],
        "place-items" => properties!["place-items"],
        "content" => properties!["--tw-content", "content"],
        "items" => properties!["align-items"],
        "justify" => properties!["justify-content"],
        "justify-items" => properties!["justify-items"],
        "place-self" => properties!["place-self"],
        "self" => properties!["align-self"],
        "justify-self" => properties!["justify-self"],
        "rounded" => properties!["border-radius"],
        "rounded-t" => properties!["border-top-left-radius", "border-top-right-radius"],
        "rounded-r" => properties!["border-top-right-radius", "border-bottom-right-radius"],
        "rounded-b" => properties!["border-bottom-right-radius", "border-bottom-left-radius"],
        "rounded-l" => properties!["border-top-left-radius", "border-bottom-left-radius"],
        "rounded-s" => properties!["border-start-start-radius", "border-end-start-radius"],
        "rounded-e" => properties!["border-start-end-radius", "border-end-end-radius"],
        "rounded-ss" => properties!["border-start-start-radius"],
        "rounded-se" => properties!["border-start-end-radius"],
        "rounded-es" => properties!["border-end-start-radius"],
        "rounded-ee" => properties!["border-end-end-radius"],
        "rounded-tl" => properties!["border-top-left-radius"],
        "rounded-tr" => properties!["border-top-right-radius"],
        "rounded-br" => properties!["border-bottom-right-radius"],
        "rounded-bl" => properties!["border-bottom-left-radius"],
        "divide" | "border" => {
            if value_is_color(value) {
                properties!["border-color"]
            } else {
                properties!["border-width"]
            }
        }
        "divide-x" => properties![
            "--tw-divide-x-reverse",
            "border-inline-style",
            "border-inline-start-width",
            "border-inline-end-width"
        ],
        "divide-y" => properties![
            "--tw-divide-y-reverse",
            "border-bottom-style",
            "border-top-style",
            "border-top-width",
            "border-bottom-width"
        ],
        "border-x" => properties!["border-inline-style", "border-inline-width"],
        "border-y" => properties!["border-block-style", "border-block-width"],
        "space-x" => {
            properties!["--tw-space-x-reverse", "margin-inline-start", "margin-inline-end"]
        }
        "space-y" => {
            properties!["--tw-space-y-reverse", "margin-block-start", "margin-block-end"]
        }
        "list" => properties!["list-style-type"],
        "list-image" => properties!["list-style-image"],
        "line-clamp" => {
            properties!["overflow", "display", "-webkit-box-orient", "-webkit-line-clamp"]
        }
        "bg" => {
            if named_value_starts_with(value, "linear-") {
                properties!["background-image"]
            } else {
                properties!["background-color"]
            }
        }
        "bg-linear" | "bg-conic" | "bg-radial" => {
            properties!["--tw-gradient-position", "background-image"]
        }
        "bg-position" => properties!["background-position"],
        "bg-size" => properties!["background-size"],
        "from" if value_is_percentage(value) => properties!["--tw-gradient-from-position"],
        "via" if value_is_percentage(value) => properties!["--tw-gradient-via-position"],
        "to" if value_is_percentage(value) => properties!["--tw-gradient-to-position"],
        "from" => properties!["--tw-gradient-from", "--tw-gradient-stops"],
        "via" => properties!["--tw-gradient-via", "--tw-gradient-via-stops"],
        "to" => properties!["--tw-gradient-to"],
        "text" => {
            if named_value_is_theme(value, "text") {
                properties!["font-size", "line-height"]
            } else {
                properties!["color"]
            }
        }
        "font" => properties!["font-family"],
        "font-stretch" => properties!["font-stretch"],
        "leading" => properties!["--tw-leading", "line-height"],
        "tracking" => properties!["--tw-tracking", "letter-spacing"],
        "underline-offset" => properties!["text-underline-offset"],
        "placeholder" => properties!["color"],
        "caret" => properties!["caret-color"],
        "accent" => properties!["accent-color"],
        "origin" => properties!["transform-origin"],
        "object" => properties!["object-position"],
        "perspective-origin" => properties!["perspective-origin"],
        "align" => properties!["vertical-align"],
        "cursor" => properties!["cursor"],
        "contain" => properties!["contain"],
        "font-features" => properties!["font-feature-settings"],
        "will-change" => properties!["will-change"],
        "transform" => properties!["transform"],
        "filter" => properties!["filter"],
        "outline-offset" => properties!["outline-offset"],
        "@container" => properties!["container-type"],
        "transition" => {
            properties!["transition-property", "transition-timing-function", "transition-duration"]
        }
        "border-s" => properties!["border-inline-start-style", "border-inline-start-width"],
        "border-e" => properties!["border-inline-end-style", "border-inline-end-width"],
        "border-bs" => properties!["border-block-start-style", "border-block-start-width"],
        "border-be" => properties!["border-block-end-style", "border-block-end-width"],
        "border-t" => properties!["border-top-style", "border-top-width"],
        "border-r" => properties!["border-right-style", "border-right-width"],
        "border-b" => properties!["border-bottom-style", "border-bottom-width"],
        "border-l" => properties!["border-left-style", "border-left-width"],
        "border-spacing" => {
            properties!["--tw-border-spacing-x", "--tw-border-spacing-y", "border-spacing"]
        }
        "border-spacing-x" => properties!["--tw-border-spacing-x", "border-spacing"],
        "border-spacing-y" => properties!["--tw-border-spacing-y", "border-spacing"],
        "opacity" => properties!["opacity"],
        "z" => properties!["z-index"],
        "order" => properties!["order"],
        "basis" => properties!["flex-basis"],
        "flex" => properties!["flex"],
        "grow" => properties!["flex-grow"],
        "shrink" => properties!["flex-shrink"],
        "translate" => properties!["translate"],
        "translate-x" => properties!["--tw-translate-x", "translate"],
        "translate-y" => properties!["--tw-translate-y", "translate"],
        "translate-z" => properties!["--tw-translate-z", "translate"],
        "rotate" => properties!["rotate"],
        "rotate-x" => properties!["--tw-rotate-x", "transform"],
        "rotate-y" => properties!["--tw-rotate-y", "transform"],
        "rotate-z" => properties!["--tw-rotate-z", "transform"],
        "scale" => properties!["--tw-scale-x", "--tw-scale-y", "--tw-scale-z", "scale"],
        "scale-x" => properties!["--tw-scale-x", "scale"],
        "scale-y" => properties!["--tw-scale-y", "scale"],
        "scale-z" => properties!["--tw-scale-z", "scale"],
        "skew" => properties!["--tw-skew-x", "--tw-skew-y", "transform"],
        "skew-x" => properties!["--tw-skew-x", "transform"],
        "skew-y" => properties!["--tw-skew-y", "transform"],
        "shadow" => properties!["--tw-shadow", "box-shadow"],
        "ring" => properties!["--tw-ring-shadow", "box-shadow"],
        "ring-offset" => properties!["--tw-ring-offset-width", "--tw-ring-offset-shadow"],
        "outline" => properties!["outline-style", "outline-width"],
        "decoration" => properties!["text-decoration-thickness"],
        "inset-shadow" => properties!["--tw-inset-shadow", "box-shadow"],
        "text-shadow" => properties!["text-shadow"],
        "inset-ring" => properties!["--tw-inset-ring-shadow", "box-shadow"],
        "drop-shadow" => properties!["--tw-drop-shadow-size", "--tw-drop-shadow", "filter"],
        "blur" => properties!["--tw-blur", "filter"],
        "brightness" => properties!["--tw-brightness", "filter"],
        "contrast" => properties!["--tw-contrast", "filter"],
        "grayscale" => properties!["--tw-grayscale", "filter"],
        "hue-rotate" => properties!["--tw-hue-rotate", "filter"],
        "invert" => properties!["--tw-invert", "filter"],
        "saturate" => properties!["--tw-saturate", "filter"],
        "sepia" => properties!["--tw-sepia", "filter"],
        "backdrop-blur" => {
            properties!["--tw-backdrop-blur", "-webkit-backdrop-filter", "backdrop-filter"]
        }
        root if root.starts_with("backdrop-") => {
            properties![format!("--tw-{}", root), "-webkit-backdrop-filter", "backdrop-filter"]
        }
        "fill" => properties!["fill"],
        "stroke" => properties!["stroke"],
        "animate" => properties!["animation"],
        "delay" => properties!["transition-delay"],
        "duration" => properties!["--tw-duration", "transition-duration"],
        "ease" => properties!["--tw-ease", "transition-timing-function"],
        "perspective" => properties!["perspective"],
        "columns" => properties!["columns"],
        "aspect" => properties!["aspect-ratio"],
        "indent" => properties!["text-indent"],
        "tab" => properties!["tab-size"],
        "zoom" => properties!["zoom"],
        "scrollbar-thumb" => properties!["--tw-scrollbar-thumb", "scrollbar-color"],
        "scrollbar-track" => properties!["--tw-scrollbar-track", "scrollbar-color"],
        "mask" => properties!["mask-image"],
        "mask-position" => properties!["mask-position"],
        "mask-size" => properties!["mask-size"],
        "mask-linear" => {
            properties![
                "mask-image",
                "mask-composite",
                "--tw-mask-linear",
                "--tw-mask-linear-position"
            ]
        }
        "mask-conic" => {
            properties![
                "mask-image",
                "mask-composite",
                "--tw-mask-conic",
                "--tw-mask-conic-position"
            ]
        }
        "mask-radial" | "mask-radial-at" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-radial",
            "--tw-mask-radial-position"
        ],
        "mask-linear-from" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-linear-stops",
            "--tw-mask-linear",
            "--tw-mask-linear-from-position"
        ],
        "mask-linear-to" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-linear-stops",
            "--tw-mask-linear",
            "--tw-mask-linear-to-position"
        ],
        "mask-conic-from" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-conic-stops",
            "--tw-mask-conic",
            "--tw-mask-conic-from-position"
        ],
        "mask-conic-to" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-conic-stops",
            "--tw-mask-conic",
            "--tw-mask-conic-to-position"
        ],
        "mask-radial-from" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-radial-stops",
            "--tw-mask-radial",
            "--tw-mask-radial-from-position"
        ],
        "mask-radial-to" => properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-radial-stops",
            "--tw-mask-radial",
            "--tw-mask-radial-to-position"
        ],
        root if root.starts_with("mask-") => mask_edge_properties(root),
        _ => Vec::new(),
    }
}

fn mask_edge_properties(root: &str) -> Vec<CompactString> {
    if let Some(stop) = root.strip_prefix("mask-x-").filter(|stop| matches!(*stop, "from" | "to")) {
        return properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-linear",
            "--tw-mask-right",
            format!("--tw-mask-right-{stop}-position"),
            "--tw-mask-left",
            format!("--tw-mask-left-{stop}-position")
        ];
    }
    if let Some(stop) = root.strip_prefix("mask-y-").filter(|stop| matches!(*stop, "from" | "to")) {
        return properties![
            "mask-image",
            "mask-composite",
            "--tw-mask-linear",
            "--tw-mask-top",
            format!("--tw-mask-top-{stop}-position"),
            "--tw-mask-bottom",
            format!("--tw-mask-bottom-{stop}-position")
        ];
    }
    let (edge, stop) = match root {
        "mask-t-from" => ("top", "from"),
        "mask-t-to" => ("top", "to"),
        "mask-r-from" => ("right", "from"),
        "mask-r-to" => ("right", "to"),
        "mask-b-from" => ("bottom", "from"),
        "mask-b-to" => ("bottom", "to"),
        "mask-l-from" => ("left", "from"),
        "mask-l-to" => ("left", "to"),
        _ => return Vec::new(),
    };
    properties![
        "mask-image",
        "mask-composite",
        "--tw-mask-linear",
        format!("--tw-mask-{edge}"),
        format!("--tw-mask-{edge}-{stop}-position")
    ]
}

fn value_is_percentage(value: Option<&CandidateValue>) -> bool {
    match value {
        Some(CandidateValue::Named(value)) => value.value.ends_with('%'),
        Some(CandidateValue::Arbitrary(value)) => value.value.ends_with('%'),
        None => false,
    }
}

fn named_value_starts_with(value: Option<&CandidateValue>, prefix: &str) -> bool {
    matches!(value, Some(CandidateValue::Named(value)) if value.value.starts_with(prefix))
}

fn named_value_is_theme(value: Option<&CandidateValue>, namespace: &str) -> bool {
    matches!(value, Some(CandidateValue::Named(value)) if value.value.starts_with(namespace))
}

fn named_value_is_color(value: Option<&CandidateValue>) -> bool {
    matches!(value, Some(CandidateValue::Named(value)) if value.value.contains('-') || matches!(value.value.as_str(), "current" | "inherit" | "transparent"))
}

fn value_is_color(value: Option<&CandidateValue>) -> bool {
    match value {
        Some(CandidateValue::Arbitrary(value)) => value_is_arbitrary_color(value),
        _ => named_value_is_color(value),
    }
}

fn value_is_arbitrary_color(value: &crate::ArbitraryValue) -> bool {
    value.data_type.as_deref() == Some("color")
        || value.value.starts_with('#')
        || value.value.starts_with("rgb(")
        || value.value.starts_with("hsl(")
        || value.value.starts_with("oklch(")
}

fn is_css_length(value: &str) -> bool {
    const UNITS: &[&str] = &[
        "px", "rem", "em", "%", "ch", "ex", "cap", "lh", "rlh", "vw", "vh", "vmin", "vmax", "svw",
        "svh", "lvw", "lvh", "dvw", "dvh", "cm", "mm", "in", "pt", "pc",
    ];
    UNITS
        .iter()
        .any(|unit| value.strip_suffix(unit).is_some_and(|number| number.parse::<f64>().is_ok()))
        || value.starts_with("calc(")
        || value.starts_with("min(")
        || value.starts_with("max(")
        || value.starts_with("clamp(")
}

#[cfg(test)]
mod tests {
    use super::functional_properties;
    use crate::{
        DesignSystem, LoadOptions,
        defaults::{FUNCTIONAL_UTILITIES, STATIC_UTILITIES},
    };

    fn design() -> DesignSystem {
        DesignSystem::load(
            &LoadOptions::new(env!("CARGO_MANIFEST_DIR"))
                .with_entry_point("tests/fixtures/base.css"),
            1,
        )
        .expect("fixture should load")
    }

    #[test]
    fn validates_classes_against_the_theme() {
        let design = design();
        assert!(design.is_known_class("bg-red-500"));
        assert!(design.is_known_class("p-2"));
        assert!(design.is_known_class("hover:block"));
        assert!(design.is_known_class("font-thin"));
        assert!(design.is_known_class("cursor-pointer"));
        assert!(design.is_known_class("divide-dashed"));
        assert!(design.is_known_class("bg-gradient-to-r"));
        assert!(design.is_known_class("-start-8"));
        assert!(design.is_known_class("border-2"));
        assert!(design.is_known_class("border-red-500"));
        assert_eq!(
            design.compile_class("border-2")[0]
                .properties
                .iter()
                .map(|property| property.css_property_name.as_str())
                .collect::<Vec<_>>(),
            ["border-style", "border-width"]
        );
        assert_eq!(
            design.compile_class("border-red-500")[0].properties[0].css_property_name,
            "border-color"
        );
        assert!(design.is_known_class("p-4.75"));
        assert!(!design.is_known_class("p-4.8"));
        assert!(design.is_known_class("opacity-12.5"));
        for class_name in
            ["translate-x-4", "-translate-y-4", "scale-x-100", "-scale-z-50", "rotate-z-45"]
        {
            assert!(design.is_known_class(class_name), "expected {class_name} to compile");
        }
        assert!(
            design.compile_class("font-thin")[0]
                .properties
                .iter()
                .any(|property| property.css_property_name == "font-weight")
        );
        assert_eq!(
            design.compile_class("border-[color:red]")[0].properties[0].css_property_name,
            "border-color"
        );
        assert!(!design.is_known_class("bg-missing-500"));
        assert!(!design.is_known_class("made-up"));
    }

    #[test]
    fn sorts_like_tailwind_property_order() {
        let design = design();
        let classes = ["text-sm", "p-2", "flex"];
        assert_eq!(design.class_order(&classes), [Some(2), Some(1), Some(0)]);
        let classes = ["cursor-pointer", "flex", "relative"];
        assert_eq!(design.class_order(&classes), [Some(2), Some(1), Some(0)]);
    }

    #[test]
    fn reports_exact_property_conflicts() {
        let design = design();
        let conflicts = design.conflicting_classes(&["p-2", "p-4", "px-2"]);
        assert!(conflicts.iter().any(|conflict| {
            conflict.class_name == "p-2" && conflict.conflicting_class_name == "p-4"
        }));
        assert!(!conflicts.iter().any(|conflict| {
            conflict.class_name == "p-2" && conflict.conflicting_class_name == "px-2"
        }));
        let conflicts =
            design.conflicting_classes(&["border-solid", "divide-solid", "divide-dashed"]);
        assert!(!conflicts.iter().any(|conflict| {
            conflict.class_name == "border-solid"
                && conflict.conflicting_class_name == "divide-solid"
        }));
        assert!(conflicts.iter().any(|conflict| {
            conflict.class_name == "divide-solid"
                && conflict.conflicting_class_name == "divide-dashed"
        }));
        let conflicts = design.conflicting_classes(&["outline", "outline-1"]);
        assert!(conflicts.iter().any(|conflict| {
            conflict.class_name == "outline" && conflict.conflicting_class_name == "outline-1"
        }));
    }

    #[test]
    fn marks_global_and_selector_variants() {
        let design = design();
        let order =
            design.variant_order(["sm:block", "print-only:block", "hocus:block", "dark:block"]);
        assert_ne!(order["sm"] & (1 << 30), 0);
        assert_ne!(order["print-only"] & (1 << 30), 0);
        assert_eq!(order["hocus"] & (1 << 30), 0);
        assert_eq!(order["dark"] & (1 << 30), 0);
        assert!(design.is_known_class("group-dark:block"));

        let defaults =
            DesignSystem::from_css("@theme { --breakpoint-lg: 64rem; --color-red-500: red; }", 1)
                .unwrap();
        let order = defaults.variant_order(["dark:lg:text-red-500"]);
        assert!(order["dark"] > order["lg"]);
        let order = defaults.variant_order(["[&.dragging]:lg:text-red-500"]);
        assert!(order["[&.dragging]"] > (order["lg"] & !(1 << 30)));
    }

    #[test]
    fn all_built_in_static_utilities_have_property_metadata() {
        let design = design();
        let missing = STATIC_UTILITIES
            .iter()
            .filter(|utility| design.compile_class(utility).is_empty())
            .collect::<Vec<_>>();
        assert!(missing.is_empty(), "missing property metadata: {missing:?}");
    }

    #[test]
    fn all_built_in_functional_utilities_have_property_metadata() {
        let missing = FUNCTIONAL_UTILITIES
            .iter()
            .filter(|utility| {
                functional_properties(utility.trim_start_matches('-'), None).is_empty()
            })
            .collect::<Vec<_>>();
        assert!(missing.is_empty(), "missing property metadata: {missing:?}");
    }
}
