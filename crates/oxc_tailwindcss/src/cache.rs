use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use rustc_hash::FxHashMap;

use crate::{DesignSystem, LoadError, LoadOptions};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    cwd: PathBuf,
    entry_point: Option<PathBuf>,
}

impl CacheKey {
    pub fn new(options: &LoadOptions) -> Self {
        Self {
            cwd: normalize(&options.cwd),
            entry_point: options.entry_point.as_ref().map(|path| {
                let path = if path.is_absolute() { path.clone() } else { options.cwd.join(path) };
                normalize(&path)
            }),
        }
    }
}

#[derive(Debug, Default)]
struct CacheState {
    entries: FxHashMap<CacheKey, Arc<DesignSystem>>,
    next_generation: u64,
}

#[derive(Debug, Default)]
pub struct DesignSystemCache {
    state: RwLock<CacheState>,
}

impl DesignSystemCache {
    /// Return a current design system, loading it when absent or stale.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the entry point or one of its transitive dependencies cannot be
    /// loaded.
    pub fn get_or_load(&self, options: &LoadOptions) -> Result<Arc<DesignSystem>, LoadError> {
        let key = CacheKey::new(options);
        let cached = {
            let state = self.read();
            state.entries.get(&key).filter(|design| design.is_current()).map(Arc::clone)
        };
        if let Some(design) = cached {
            return Ok(design);
        }

        let mut state = self.write();
        if let Some(design) =
            state.entries.get(&key).filter(|design| design.is_current()).map(Arc::clone)
        {
            return Ok(design);
        }
        state.next_generation += 1;
        let design = Arc::new(DesignSystem::load(options, state.next_generation)?);
        state.entries.insert(key, Arc::clone(&design));
        Ok(design)
    }

    /// Invalidate every design system whose transitive dependency graph contains `path`.
    pub fn invalidate_path(&self, path: &Path) -> usize {
        let path = normalize(path);
        let mut state = self.write();
        let before = state.entries.len();
        state.entries.retain(|_, design| {
            !design.dependencies().iter().any(|dependency| normalize(&dependency.path) == path)
        });
        before - state.entries.len()
    }

    pub fn clear(&self) {
        self.write().entries.clear();
    }

    fn read(&self) -> RwLockReadGuard<'_, CacheState> {
        self.state.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, CacheState> {
        self.state.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
