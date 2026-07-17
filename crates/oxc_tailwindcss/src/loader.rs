use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    fs, io,
    path::{Path, PathBuf},
};

use compact_str::CompactString;
use rustc_hash::FxHashSet;

use crate::{
    Dependency, DependencyStamp, DesignSystem, LoadOptions, VariantRegistration,
    design_system::Compounds,
};

#[derive(Debug)]
pub enum LoadError {
    Io { path: PathBuf, source: io::Error },
    TailwindPackageNotFound { cwd: PathBuf },
    ImportNotFound { request: String, base: PathBuf },
    JavaScriptModuleUnsupported { directive: &'static str, path: PathBuf },
}

impl Display for LoadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::TailwindPackageNotFound { cwd } => {
                write!(formatter, "could not find node_modules/tailwindcss from {}", cwd.display())
            }
            Self::ImportNotFound { request, base } => {
                write!(
                    formatter,
                    "could not resolve CSS import {request:?} from {}",
                    base.display()
                )
            }
            Self::JavaScriptModuleUnsupported { directive, path } => write!(
                formatter,
                "{directive} in {} requires JavaScript execution and is not supported by the native design system",
                path.display()
            ),
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

struct LoadedCss {
    path: PathBuf,
    content: String,
}

pub fn load_design_system(
    options: &LoadOptions,
    generation: u64,
) -> Result<DesignSystem, LoadError> {
    let cwd = canonical_or_original(&options.cwd);
    let mut resolution_dependencies = FxHashSet::default();
    let entry = match &options.entry_point {
        Some(entry) => {
            let entry = if entry.is_absolute() { entry.clone() } else { cwd.join(entry) };
            canonical_or_original(&entry)
        }
        None => resolve_package_file(&cwd, "tailwindcss/index.css", &mut resolution_dependencies)
            .ok_or_else(|| LoadError::TailwindPackageNotFound { cwd: cwd.clone() })?,
    };

    let mut visited = FxHashSet::default();
    let mut files = Vec::new();
    load_css_recursive(&entry, &cwd, &mut visited, &mut files, &mut resolution_dependencies)?;
    let dependency_paths = files
        .iter()
        .map(|file| file.path.clone())
        .chain(resolution_dependencies)
        .collect::<FxHashSet<_>>();
    let dependencies = dependency_paths
        .into_iter()
        .map(|path| {
            DependencyStamp::read(&path)
                .map(|stamp| Dependency { path: path.clone(), stamp })
                .map_err(|source| LoadError::Io { path, source })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut design = DesignSystem::new(generation, dependencies);
    for file in files {
        apply_css_configuration(&mut design, &file.path, &file.content)?;
    }
    design.register_theme_variants();
    Ok(design)
}

pub fn load_design_system_from_css(
    content: &str,
    generation: u64,
) -> Result<DesignSystem, LoadError> {
    let mut design = DesignSystem::new(generation, Vec::new());
    apply_css_configuration(&mut design, Path::new("<memory>"), content)?;
    design.register_theme_variants();
    Ok(design)
}

fn load_css_recursive(
    path: &Path,
    cwd: &Path,
    visited: &mut FxHashSet<PathBuf>,
    files: &mut Vec<LoadedCss>,
    resolution_dependencies: &mut FxHashSet<PathBuf>,
) -> Result<(), LoadError> {
    let path = canonical_or_original(path);
    if !visited.insert(path.clone()) {
        return Ok(());
    }
    let content =
        fs::read_to_string(&path).map_err(|source| LoadError::Io { path: path.clone(), source })?;
    let base = path.parent().unwrap_or(cwd);
    for import in stylesheet_references(&content) {
        if import.request.starts_with("url(")
            || import.request.starts_with("http:")
            || import.request.starts_with("https:")
        {
            continue;
        }
        let imported = resolve_import(&import.request, base, cwd, resolution_dependencies)
            .ok_or_else(|| LoadError::ImportNotFound {
                request: import.request.clone(),
                base: base.to_path_buf(),
            })?;
        load_css_recursive(&imported, cwd, visited, files, resolution_dependencies)?;
    }
    files.push(LoadedCss { path, content });
    Ok(())
}

struct Import {
    request: String,
}

fn stylesheet_references(content: &str) -> Vec<Import> {
    let bytes = content.as_bytes();
    let mut imports = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            index = content[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |relative| index + relative + 4);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"') {
            index = skip_css_string(bytes, index);
            continue;
        }
        let keyword = ["@import", "@reference"].into_iter().find(|keyword| {
            content[index..].starts_with(keyword)
                && bytes.get(index + keyword.len()).is_none_or(u8::is_ascii_whitespace)
        });
        let Some(keyword) = keyword else {
            index = advance_char(content, index);
            continue;
        };
        index = skip_whitespace(bytes, index + keyword.len());
        if let Some((request, end)) = parse_css_string(content, index) {
            imports.push(Import { request: request.to_owned() });
            index = end;
        } else {
            index = advance_char(content, index);
        }
    }
    imports
}

fn skip_css_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn resolve_import(
    request: &str,
    base: &Path,
    cwd: &Path,
    resolution_dependencies: &mut FxHashSet<PathBuf>,
) -> Option<PathBuf> {
    if request.starts_with('.') || request.starts_with('/') {
        let path = if Path::new(request).is_absolute() {
            PathBuf::from(request)
        } else {
            base.join(request)
        };
        return resolve_css_extension(&path);
    }
    resolve_package_file(base, request, resolution_dependencies)
        .or_else(|| resolve_package_file(cwd, request, resolution_dependencies))
}

fn resolve_package_file(
    base: &Path,
    request: &str,
    resolution_dependencies: &mut FxHashSet<PathBuf>,
) -> Option<PathBuf> {
    let (package, subpath) = package_and_subpath(request);
    for ancestor in base.ancestors() {
        let root = ancestor.join("node_modules").join(package);
        if !root.is_dir() {
            continue;
        }
        let manifest = root.join("package.json");
        if manifest.is_file() {
            resolution_dependencies.insert(canonical_or_original(&manifest));
        }
        if let Some(path) = resolve_package_export(&root, subpath) {
            return Some(path);
        }
        let relative = if subpath.is_empty() { "index.css" } else { subpath };
        if let Some(path) = resolve_css_extension(&root.join(relative)) {
            return Some(path);
        }
    }
    None
}

fn resolve_package_export(root: &Path, subpath: &str) -> Option<PathBuf> {
    let manifest = fs::read_to_string(root.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let key = if subpath.is_empty() { ".".to_owned() } else { format!("./{subpath}") };
    if let Some(exports) = manifest.get("exports") {
        let export = exports.get(&key).or_else(|| (key == ".").then_some(exports));
        if let Some(target) = export.and_then(css_export_target)
            && let Some(path) = resolve_css_extension(&root.join(target.trim_start_matches("./")))
        {
            return Some(path);
        }
    }
    if subpath.is_empty()
        && let Some(style) = manifest.get("style").and_then(serde_json::Value::as_str)
    {
        return resolve_css_extension(&root.join(style));
    }
    None
}

fn css_export_target(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::String(target) => Some(target),
        serde_json::Value::Object(conditions) => ["style", "default", "import", "require"]
            .into_iter()
            .find_map(|condition| conditions.get(condition).and_then(css_export_target)),
        serde_json::Value::Array(targets) => targets.iter().find_map(css_export_target),
        _ => None,
    }
}

fn package_and_subpath(request: &str) -> (&str, &str) {
    if request.starts_with('@') {
        let mut slashes = request.match_indices('/');
        let Some((_, _)) = slashes.next() else { return (request, "") };
        let Some((index, _)) = slashes.next() else { return (request, "") };
        (&request[..index], &request[index + 1..])
    } else if let Some(index) = request.find('/') {
        (&request[..index], &request[index + 1..])
    } else {
        (request, "")
    }
}

fn resolve_css_extension(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(canonical_or_original(path));
    }
    if path.extension().is_none() {
        let css = path.with_extension("css");
        if css.is_file() {
            return Some(canonical_or_original(&css));
        }
    }
    None
}

fn apply_css_configuration(
    design: &mut DesignSystem,
    path: &Path,
    content: &str,
) -> Result<(), LoadError> {
    for directive in ["@plugin", "@config"] {
        if contains_at_rule(content, directive) {
            return Err(LoadError::JavaScriptModuleUnsupported {
                directive,
                path: path.to_path_buf(),
            });
        }
    }

    if let Some(prefix) = find_function_argument(content, "prefix")
        && !prefix.is_empty()
        && prefix.bytes().all(|byte| byte.is_ascii_lowercase())
    {
        design.prefix = Some(CompactString::new(prefix));
    }

    for block in at_rule_blocks(content, "theme") {
        for declaration in declarations(block.body) {
            if declaration.name.starts_with("--") {
                if declaration.value == "initial" {
                    if declaration.name == "--*" {
                        design.theme.clear();
                    } else if let Some(namespace) = declaration.name.strip_suffix('*') {
                        design.theme.retain(|name, _| !name.starts_with(namespace));
                    } else {
                        design.theme.remove(declaration.name);
                    }
                } else {
                    design.theme.insert(
                        CompactString::new(declaration.name),
                        CompactString::new(declaration.value),
                    );
                }
            }
        }
    }

    for block in at_rule_blocks(content, "utility") {
        let name = block.prelude.split_ascii_whitespace().next().unwrap_or_default();
        let mut properties = declarations(block.body)
            .into_iter()
            .filter(|declaration| is_css_property_name(declaration.name))
            .map(|declaration| CompactString::new(declaration.name))
            .collect::<Vec<_>>();
        if properties.is_empty() && !name.is_empty() {
            properties.push(CompactString::new(format!("--tw-custom-{name}")));
        }
        if let Some(root) = name.strip_suffix("-*") {
            design.functional_utilities.insert(CompactString::new(root));
            design.custom_utilities.insert(CompactString::new(root), properties);
        } else if !name.is_empty() {
            design.static_utilities.insert(CompactString::new(name));
            design.custom_utilities.insert(CompactString::new(name), properties);
        }
    }

    for block in at_rule_blocks(content, "layer") {
        if block.prelude.split_ascii_whitespace().next() == Some("components") {
            for class_name in css_class_selectors(block.body) {
                design.component_classes.insert(CompactString::new(class_name));
            }
        }
    }

    for block in at_rule_blocks(content, "custom-variant") {
        let name = block.prelude.split_ascii_whitespace().next().unwrap_or_default();
        if !name.is_empty() {
            let order = design.next_variant_order();
            let is_global = custom_variant_is_global(block.body);
            design.variants.insert(
                CompactString::new(name),
                VariantRegistration::Static {
                    order,
                    compounds: if is_global { Compounds::AT_RULES } else { Compounds::STYLE_RULES },
                },
            );
            if is_global {
                design.global_variants.insert(CompactString::new(name));
                design.local_variants.remove(name);
            } else {
                design.local_variants.insert(CompactString::new(name));
                design.global_variants.remove(name);
            }
        }
    }
    for prelude in at_rule_statements(content, "custom-variant") {
        let name = prelude.split_ascii_whitespace().next().unwrap_or_default();
        if !name.is_empty() {
            let order = design.next_variant_order();
            let is_global =
                custom_variant_is_global(prelude.strip_prefix(name).unwrap_or_default());
            design.variants.insert(
                CompactString::new(name),
                VariantRegistration::Static {
                    order,
                    compounds: if is_global { Compounds::AT_RULES } else { Compounds::STYLE_RULES },
                },
            );
            if is_global {
                design.global_variants.insert(CompactString::new(name));
                design.local_variants.remove(name);
            } else {
                design.local_variants.insert(CompactString::new(name));
                design.global_variants.remove(name);
            }
        }
    }
    Ok(())
}

fn css_class_selectors(content: &str) -> Vec<&str> {
    let bytes = content.as_bytes();
    let mut classes = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            index = content[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |relative| index + relative + 4);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"') {
            index = skip_css_string(bytes, index);
            continue;
        }
        if bytes[index] != b'.' || bytes.get(index.wrapping_sub(1)).is_some_and(u8::is_ascii_digit)
        {
            index = advance_char(content, index);
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while bytes
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            end += 1;
        }
        let belongs_to_selector = content[end..]
            .find(['{', ';'])
            .is_some_and(|relative| content.as_bytes()[end + relative] == b'{');
        if end > start && !bytes[start].is_ascii_digit() && belongs_to_selector {
            classes.push(&content[start..end]);
        }
        index = end.max(index + 1);
    }
    classes
}

fn is_css_property_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'-'
                || name.starts_with("--") && (byte.is_ascii_uppercase() || byte == b'_')
        })
}

fn contains_at_rule(content: &str, directive: &str) -> bool {
    let bytes = content.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            index = content[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |relative| index + relative + 4);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"') {
            index = skip_css_string(bytes, index);
            continue;
        }
        if content[index..].starts_with(directive)
            && bytes.get(index + directive.len()).is_none_or(|byte| {
                byte.is_ascii_whitespace() || matches!(byte, b'\'' | b'"' | b';' | b'{')
            })
        {
            return true;
        }
        index = advance_char(content, index);
    }
    false
}

fn custom_variant_is_global(body: &str) -> bool {
    let body =
        body.trim().strip_prefix('(').and_then(|body| body.strip_suffix(')')).unwrap_or(body);
    !body.contains('&')
}

struct AtRuleBlock<'a> {
    prelude: &'a str,
    body: &'a str,
}

fn at_rule_blocks<'a>(content: &'a str, name: &str) -> Vec<AtRuleBlock<'a>> {
    let needle = format!("@{name}");
    let mut blocks = Vec::new();
    let mut index = 0;
    while let Some(rule_start) = next_at_rule(content, &needle, index) {
        let start = rule_start + needle.len();
        let Some(delimiter_relative) = content[start..].find([';', '{']) else { break };
        let delimiter = start + delimiter_relative;
        if content.as_bytes()[delimiter] == b';' {
            index = delimiter + 1;
            continue;
        }
        let open = delimiter;
        let Some(close) = matching_brace(content, open) else { break };
        blocks.push(AtRuleBlock {
            prelude: content[start..open].trim(),
            body: &content[open + 1..close],
        });
        index = close + 1;
    }
    blocks
}

fn at_rule_statements<'a>(content: &'a str, name: &str) -> Vec<&'a str> {
    let needle = format!("@{name}");
    let mut statements = Vec::new();
    let mut index = 0;
    while let Some(rule_start) = next_at_rule(content, &needle, index) {
        let start = rule_start + needle.len();
        let Some(end_relative) = content[start..].find([';', '{']) else { break };
        let end = start + end_relative;
        if content.as_bytes()[end] == b';' {
            statements.push(content[start..end].trim());
        }
        index = end + 1;
    }
    statements
}

fn next_at_rule(content: &str, needle: &str, mut index: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            index = content[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |relative| index + relative + 4);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"') {
            index = skip_css_string(bytes, index);
            continue;
        }
        if content[index..].starts_with(needle)
            && bytes
                .get(index + needle.len())
                .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b';' | b'{'))
        {
            return Some(index);
        }
        index = advance_char(content, index);
    }
    None
}

struct Declaration<'a> {
    name: &'a str,
    value: &'a str,
}

fn declarations(content: &str) -> Vec<Declaration<'_>> {
    crate::segment::segment(content, b';')
        .into_iter()
        .filter_map(|part| {
            let part = trim_leading_css_comments(part);
            let (name, value) = part.split_once(':')?;
            let name = name.trim();
            let value = value.trim();
            (!name.is_empty() && !value.is_empty()).then_some(Declaration { name, value })
        })
        .collect()
}

fn trim_leading_css_comments(mut input: &str) -> &str {
    loop {
        input = input.trim_start();
        let Some(comment) = input.strip_prefix("/*") else { return input };
        let Some(end) = comment.find("*/") else { return "" };
        input = &comment[end + 2..];
    }
}

fn matching_brace(content: &str, open: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut depth = 0_u32;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            quote @ (b'\'' | b'"') => {
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index += 2,
                        byte if byte == quote => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn find_function_argument<'a>(content: &'a str, function: &str) -> Option<&'a str> {
    let needle = format!("{function}(");
    let bytes = content.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            index = content[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |relative| index + relative + 4);
            continue;
        }
        if matches!(bytes[index], b'\'' | b'"') {
            index = skip_css_string(bytes, index);
            continue;
        }
        if content[index..].starts_with(&needle) {
            let start = index + needle.len();
            let end = content[start..].find(')')? + start;
            return Some(content[start..end].trim());
        }
        index = advance_char(content, index);
    }
    None
}

fn parse_css_string(content: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = content.as_bytes();
    let quote = *bytes.get(start)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return Some((&content[start + 1..index], index + 1)),
            _ => index += 1,
        }
    }
    None
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    index
}

fn advance_char(content: &str, index: usize) -> usize {
    index + content[index..].chars().next().map_or(1, char::len_utf8)
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::{DesignSystemCache, LoadOptions, UtilityKind};

    #[test]
    fn loads_transitive_css_and_invalidates() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("app.css");
        let theme = directory.path().join("theme.css");
        let reference = directory.path().join("reference.css");
        fs::write(
            &root,
            "/* @import './missing.css'; @plugin './ignored.js'; */ @import './theme.css'; @reference './reference.css'; \
             @utility tab-* { tab-size: --value(integer); } \
             @layer components { .card:hover { color: red; } }",
        )
        .unwrap();
        fs::write(&theme, "@theme { --color-brand: oklch(50% .2 20); }").unwrap();
        fs::write(&reference, "@theme { --breakpoint-wide: 80rem; }").unwrap();

        let options = LoadOptions::new(directory.path()).with_entry_point(&root);
        let cache = DesignSystemCache::default();
        let first = cache.get_or_load(&options).unwrap();
        assert_eq!(first.theme_value("--color-brand"), Some("oklch(50% .2 20)"));
        assert_eq!(first.utility_kind("tab"), Some(UtilityKind::Functional));
        assert_eq!(first.dependencies().len(), 3);
        assert!(first.variant_registration("wide").is_some());
        assert!(first.variant_is_global("wide"));
        assert!(first.has_component_class("card"));

        assert_eq!(cache.invalidate_path(&theme), 1);
        fs::write(&theme, "@theme { --color-brand: red; }").unwrap();
        let second = cache.get_or_load(&options).unwrap();
        assert!(second.generation() > first.generation());
        assert_eq!(second.theme_value("--color-brand"), Some("red"));

        // Callers do not have to explicitly invalidate: every cache hit validates the complete
        // dependency graph first. Use a different file length so this remains reliable on file
        // systems with coarse modification timestamps.
        fs::write(&theme, "@theme { --color-brand: rebeccapurple; }").unwrap();
        let third = cache.get_or_load(&options).unwrap();
        assert!(third.generation() > second.generation());
        assert_eq!(third.theme_value("--color-brand"), Some("rebeccapurple"));
    }

    #[test]
    fn resolves_package_style_exports() {
        let directory = tempdir().unwrap();
        let package = directory.path().join("node_modules/theme-package");
        fs::create_dir_all(package.join("styles")).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"exports":{".":{"style":"./styles/index.css"}}}"#,
        )
        .unwrap();
        fs::write(package.join("styles/index.css"), "@theme { --color-package: blue; }").unwrap();
        fs::write(package.join("styles/alternate.css"), "@theme { --color-package: orange; }")
            .unwrap();
        let root = directory.path().join("app.css");
        fs::write(&root, "@import 'theme-package';").unwrap();

        let options = LoadOptions::new(directory.path()).with_entry_point(root);
        let cache = DesignSystemCache::default();
        let first = cache.get_or_load(&options).unwrap();
        assert_eq!(first.theme_value("--color-package"), Some("blue"));
        assert_eq!(first.dependencies().len(), 3);

        // Package exports participate in resolution, so changing only package.json must rebuild
        // the design system and follow the new stylesheet target.
        fs::write(
            package.join("package.json"),
            r#"{"exports":{".":{"style":"./styles/alternate.css"}},"version":"2"}"#,
        )
        .unwrap();
        let second = cache.get_or_load(&options).unwrap();
        assert!(second.generation() > first.generation());
        assert_eq!(second.theme_value("--color-package"), Some("orange"));
    }
}
