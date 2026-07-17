use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::SystemTime,
};

use bitflags::bitflags;
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    Candidate, CompiledRule, LoadError, Variant,
    defaults::{FUNCTIONAL_UTILITIES, NEGATIVE_FUNCTIONAL_UTILITIES, STATIC_UTILITIES},
    loader::{load_design_system, load_design_system_from_css},
};

pub const ARBITRARY_VARIANT_ORDER: u32 = 1024;
pub const MAX_VARIANT_ORDER: u32 = 100;
pub const THEME_VARIANT_ORDER: u32 = 200;
pub const THEME_VARIANT_RANGE: u32 = 64;

const MAX_CACHED_CLASSES: usize = 4096;

#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub cwd: PathBuf,
    pub entry_point: Option<PathBuf>,
}

impl LoadOptions {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into(), entry_point: None }
    }

    #[must_use]
    pub fn with_entry_point(mut self, entry_point: impl Into<PathBuf>) -> Self {
        self.entry_point = Some(entry_point.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl DependencyStamp {
    /// Read a stamp used to detect whether a dependency changed.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if metadata for `path` cannot be read.
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self { modified: metadata.modified().ok(), len: metadata.len() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub path: PathBuf,
    pub stamp: DependencyStamp,
}

impl Dependency {
    pub fn is_current(&self) -> bool {
        DependencyStamp::read(&self.path).is_ok_and(|stamp| stamp == self.stamp)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtilityKind {
    Static,
    Functional,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Compounds: u8 {
        const STYLE_RULES = 1 << 0;
        const AT_RULES = 1 << 1;
        const NOT = 1 << 2;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantRegistration {
    Static { order: u32, compounds: Compounds },
    Functional { order: u32, compounds: Compounds },
    Compound { order: u32, compounds: Compounds, compounds_with: Compounds },
}

impl VariantRegistration {
    pub fn order(self) -> u32 {
        match self {
            Self::Static { order, .. }
            | Self::Functional { order, .. }
            | Self::Compound { order, .. } => order,
        }
    }

    pub fn compounds(self) -> Compounds {
        match self {
            Self::Static { compounds, .. }
            | Self::Functional { compounds, .. }
            | Self::Compound { compounds, .. } => compounds,
        }
    }

    pub fn compounds_with(self) -> Compounds {
        match self {
            Self::Compound { compounds_with, .. } => compounds_with,
            Self::Static { .. } | Self::Functional { .. } => Compounds::empty(),
        }
    }
}

#[derive(Debug)]
pub struct DesignSystem {
    pub(crate) prefix: Option<CompactString>,
    pub(crate) theme: FxHashMap<CompactString, CompactString>,
    pub(crate) static_utilities: FxHashSet<CompactString>,
    pub(crate) functional_utilities: FxHashSet<CompactString>,
    pub(crate) variants: FxHashMap<CompactString, VariantRegistration>,
    pub(crate) global_variants: FxHashSet<CompactString>,
    pub(crate) local_variants: FxHashSet<CompactString>,
    pub(crate) custom_utilities: FxHashMap<CompactString, Vec<CompactString>>,
    pub(crate) component_classes: FxHashSet<CompactString>,
    pub(crate) candidate_cache: RwLock<FxHashMap<CompactString, Arc<[Candidate]>>>,
    pub(crate) compiled_cache: RwLock<FxHashMap<CompactString, Arc<[CompiledRule]>>>,
    dependencies: Vec<Dependency>,
    generation: u64,
}

impl DesignSystem {
    /// Load a native design system and its transitive CSS dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the CSS entry point cannot be resolved or parsed.
    pub fn load(options: &LoadOptions, generation: u64) -> Result<Self, LoadError> {
        load_design_system(options, generation)
    }

    /// Build a design system from an in-memory CSS entry point.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the stylesheet references unsupported JavaScript configuration.
    pub fn from_css(css: &str, generation: u64) -> Result<Self, LoadError> {
        load_design_system_from_css(css, generation)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    pub fn is_current(&self) -> bool {
        self.dependencies.iter().all(Dependency::is_current)
    }

    pub fn theme_value(&self, name: &str) -> Option<&str> {
        self.theme.get(name).map(CompactString::as_str)
    }

    pub fn utility_kind(&self, root: &str) -> Option<UtilityKind> {
        if self.has_static_utility(root) {
            Some(UtilityKind::Static)
        } else if self.has_functional_utility(root) {
            Some(UtilityKind::Functional)
        } else {
            None
        }
    }

    pub fn has_static_utility(&self, root: &str) -> bool {
        self.static_utilities.contains(root)
    }

    pub fn has_functional_utility(&self, root: &str) -> bool {
        self.functional_utilities.contains(root)
            || root
                .strip_prefix('-')
                .is_some_and(|root| NEGATIVE_FUNCTIONAL_UTILITIES.binary_search(&root).is_ok())
    }

    pub fn variant_registration(&self, root: &str) -> Option<VariantRegistration> {
        self.variants.get(root).copied()
    }

    pub fn custom_utility_properties(&self, root: &str) -> Option<&[CompactString]> {
        self.custom_utilities.get(root).map(Vec::as_slice)
    }

    pub fn has_component_class(&self, class_name: &str) -> bool {
        self.component_classes.contains(class_name)
    }

    pub fn parse_candidate(&self, input: &str) -> Vec<Candidate> {
        self.cached_candidates(input).as_ref().to_vec()
    }

    pub(crate) fn cached_candidates(&self, input: &str) -> Arc<[Candidate]> {
        if let Some(candidates) = self
            .candidate_cache
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(input)
        {
            return Arc::clone(candidates);
        }

        let candidates: Arc<[Candidate]> = Candidate::parse(input, self).into();
        let mut cache =
            self.candidate_cache.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= MAX_CACHED_CLASSES {
            cache.clear();
        }
        Arc::clone(cache.entry(CompactString::new(input)).or_insert(candidates))
    }

    pub fn parse_variant(&self, input: &str) -> Option<Variant> {
        Variant::parse(input, self)
    }

    pub(crate) fn new(generation: u64, dependencies: Vec<Dependency>) -> Self {
        let mut design = Self {
            prefix: None,
            theme: FxHashMap::default(),
            static_utilities: FxHashSet::default(),
            functional_utilities: FxHashSet::default(),
            variants: FxHashMap::default(),
            global_variants: FxHashSet::default(),
            local_variants: FxHashSet::default(),
            custom_utilities: FxHashMap::default(),
            component_classes: FxHashSet::default(),
            candidate_cache: RwLock::new(FxHashMap::default()),
            compiled_cache: RwLock::new(FxHashMap::default()),
            dependencies,
            generation,
        };
        design.register_defaults();
        design
    }

    fn register_defaults(&mut self) {
        for utility in STATIC_UTILITIES {
            self.static_utilities.insert(CompactString::new(utility));
        }
        for utility in FUNCTIONAL_UTILITIES {
            self.functional_utilities.insert(CompactString::new(utility));
        }
        for utility in NEGATIVE_FUNCTIONAL_UTILITIES {
            self.functional_utilities.insert(CompactString::new(utility));
        }

        let style = Compounds::STYLE_RULES;
        for variant in [
            "*",
            "**",
            "first-letter",
            "first-line",
            "marker",
            "selection",
            "file",
            "placeholder",
            "backdrop",
            "details-content",
            "before",
            "after",
            "first",
            "last",
            "only",
            "odd",
            "even",
            "first-of-type",
            "last-of-type",
            "only-of-type",
            "visited",
            "target",
            "open",
            "default",
            "checked",
            "indeterminate",
            "placeholder-shown",
            "autofill",
            "optional",
            "required",
            "valid",
            "invalid",
            "user-valid",
            "user-invalid",
            "in-range",
            "out-of-range",
            "read-only",
            "empty",
            "focus-within",
            "hover",
            "focus",
            "focus-visible",
            "active",
            "enabled",
            "disabled",
            "inert",
            "motion-safe",
            "motion-reduce",
            "contrast-more",
            "contrast-less",
            "portrait",
            "landscape",
            "ltr",
            "rtl",
            "dark",
            "starting",
            "print",
            "forced-colors",
            "inverted-colors",
            "pointer-none",
            "pointer-coarse",
            "pointer-fine",
            "any-pointer-none",
            "any-pointer-coarse",
            "any-pointer-fine",
            "noscript",
        ] {
            let compounds =
                if Self::builtin_variant_is_global(variant) { Compounds::AT_RULES } else { style };
            self.variants.insert(
                CompactString::new(variant),
                VariantRegistration::Static { order: builtin_variant_order(variant), compounds },
            );
        }
        for variant in [
            "aria",
            "data",
            "nth",
            "nth-last",
            "nth-of-type",
            "nth-last-of-type",
            "supports",
            "max",
            "min",
            "@max",
            "@",
            "@min",
        ] {
            let compounds = if matches!(variant, "supports" | "max" | "min" | "@max" | "@" | "@min")
            {
                Compounds::AT_RULES
            } else {
                style
            };
            self.variants.insert(
                CompactString::new(variant),
                VariantRegistration::Functional {
                    order: builtin_variant_order(variant),
                    compounds,
                },
            );
        }
        for variant in ["not", "group", "peer", "has", "in"] {
            let compounds_with =
                if variant == "not" { Compounds::STYLE_RULES | Compounds::AT_RULES } else { style };
            self.variants.insert(
                CompactString::new(variant),
                VariantRegistration::Compound {
                    order: builtin_variant_order(variant),
                    compounds: style,
                    compounds_with,
                },
            );
        }
    }

    pub fn variant_is_global(&self, root: &str) -> bool {
        !self.local_variants.contains(root)
            && (self.global_variants.contains(root) || Self::builtin_variant_is_global(root))
    }

    fn builtin_variant_is_global(root: &str) -> bool {
        matches!(
            root,
            "motion-safe"
                | "motion-reduce"
                | "contrast-more"
                | "contrast-less"
                | "portrait"
                | "landscape"
                | "dark"
                | "starting"
                | "print"
                | "forced-colors"
                | "inverted-colors"
                | "pointer-none"
                | "pointer-coarse"
                | "pointer-fine"
                | "any-pointer-none"
                | "any-pointer-coarse"
                | "any-pointer-fine"
                | "noscript"
                | "min"
                | "max"
                | "@min"
                | "@max"
        )
    }

    pub(crate) fn next_variant_order(&self) -> u32 {
        self.variants
            .values()
            .map(|registration| registration.order())
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub(crate) fn register_theme_variants(&mut self) {
        let mut variants = self
            .theme
            .iter()
            .filter_map(|(name, value)| {
                name.strip_prefix("--breakpoint-")
                    .map(|name| (CompactString::new(name), value.clone()))
            })
            .collect::<Vec<_>>();
        variants.sort_by(|(_, left), (_, right)| compare_breakpoints(left, right));
        for (index, (name, _)) in variants.into_iter().enumerate() {
            let order = THEME_VARIANT_ORDER
                + u32::try_from(index).expect("breakpoint variant index should fit u32");
            self.variants
                .entry(name.clone())
                .or_insert(VariantRegistration::Static { order, compounds: Compounds::AT_RULES });
            self.global_variants.insert(name);
        }
    }
}

fn builtin_variant_order(root: &str) -> u32 {
    match root {
        "*" => 0,
        "**" => 1,
        "not" => 2,
        "group" => 3,
        "peer" => 4,
        "in" => 80,
        "has" => 81,
        "aria" => 82,
        "data" => 83,
        "nth" => 84,
        "nth-last" => 85,
        "nth-of-type" => 86,
        "nth-last-of-type" => 87,
        "supports" => 88,
        "motion-safe" => 90,
        "motion-reduce" => 91,
        "contrast-more" => 92,
        "contrast-less" => 93,
        "max" => MAX_VARIANT_ORDER,
        "min" => THEME_VARIANT_ORDER,
        "@max" => 300,
        "@" | "@min" => 320,
        "portrait" => 400,
        "landscape" => 401,
        "ltr" => 402,
        "rtl" => 403,
        "dark" => 404,
        "starting" => 405,
        "print" => 406,
        "forced-colors" => 407,
        "inverted-colors" => 408,
        "pointer-none" => 409,
        "pointer-coarse" => 410,
        "pointer-fine" => 411,
        "any-pointer-none" => 412,
        "any-pointer-coarse" => 413,
        "any-pointer-fine" => 414,
        "noscript" => 415,
        _ => {
            const CORE: &[&str] = &[
                "first-letter",
                "first-line",
                "marker",
                "selection",
                "file",
                "placeholder",
                "backdrop",
                "details-content",
                "before",
                "after",
                "first",
                "last",
                "only",
                "odd",
                "even",
                "first-of-type",
                "last-of-type",
                "only-of-type",
                "visited",
                "target",
                "open",
                "default",
                "checked",
                "indeterminate",
                "placeholder-shown",
                "autofill",
                "optional",
                "required",
                "valid",
                "invalid",
                "user-valid",
                "user-invalid",
                "in-range",
                "out-of-range",
                "read-only",
                "empty",
                "focus-within",
                "hover",
                "focus",
                "focus-visible",
                "active",
                "enabled",
                "disabled",
                "inert",
            ];
            CORE.iter()
                .position(|variant| *variant == root)
                .map_or(ARBITRARY_VARIANT_ORDER - 1, |index| {
                    10 + u32::try_from(index).expect("builtin variant index should fit u32")
                })
        }
    }
}

fn compare_breakpoints(left: &str, right: &str) -> std::cmp::Ordering {
    let left_function = left.find('(');
    let right_function = right.find('(');
    let left_bucket = left_function.map_or_else(
        || left.trim_matches(|character: char| character.is_ascii_digit() || character == '.'),
        |index| &left[..index],
    );
    let right_bucket = right_function.map_or_else(
        || right.trim_matches(|character: char| character.is_ascii_digit() || character == '.'),
        |index| &right[..index],
    );
    left_bucket.cmp(right_bucket).then_with(|| {
        leading_number(left)
            .zip(leading_number(right))
            .and_then(|(left, right)| left.partial_cmp(&right))
            .unwrap_or_else(|| left.cmp(right))
    })
}

fn leading_number(value: &str) -> Option<f64> {
    let end = value
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
        .map(|(index, character)| index + character.len_utf8())
        .last()?;
    value[..end].parse().ok()
}
