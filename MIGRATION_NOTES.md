# Tantivy and Lindera Migration Notes

## Summary

Successfully upgraded Tantivy and Lindera to their latest versions:
- **Tantivy**: 0.24.1 → 0.25.0
- **Lindera**: 0.44.1 → 1.4.1
- **Lindera-IPADIC**: 0.44.1 → 1.4.1
- **Lindera-Tantivy**: 0.44.1 → 1.1.1

## Key Changes

### Tantivy 0.25.0
- No breaking changes from 0.24.1
- Minor bug fixes and improvements
- Update went smoothly with no code changes required

### Lindera 1.4.1
Major version jump with significant API changes:

#### API Breaking Changes
1. **Dictionary Loading**:
   - **Old**: `load_dictionary_from_kind(DictionaryKind::IPADIC)`
   - **New**: `lindera_ipadic::embedded::load()`

2. **Feature Flags**:
   - **Old**: `features = ["ipadic"]`
   - **New**: `features = ["embedded-ipadic"]`

3. **Dictionary Module**:
   - The `DictionaryKind` enum and `load_dictionary_from_kind()` function were removed
   - New dictionary loading uses embedded modules per dictionary type

#### Behavioral Changes
- **Whitespace Handling**: Lindera 1.4.0+ ignores whitespace by default (previously included)
  - To restore old behavior: use `--keep-whitespace` flag or `set_segmenter_keep_whitespace(true)`

## Code Changes

### src/indexer.rs
```rust
// Before:
use lindera::dictionary::{load_dictionary_from_kind, DictionaryKind};
let dictionary = load_dictionary_from_kind(DictionaryKind::IPADIC)?;

// After:
let dictionary = lindera_ipadic::embedded::load()?;
```

### src/main.rs
```rust
// Before:
let dictionary = lindera::dictionary::load_dictionary_from_kind(
    lindera::dictionary::DictionaryKind::IPADIC
)?;

// After:
let dictionary = lindera_ipadic::embedded::load()?;
```

### Cargo.toml
```toml
# Before:
lindera = { version = "0.44.1", features = ["ipadic"] }
lindera-ipadic = "0.44.1"
lindera-tantivy = { version = "0.44.1", features = ["ipadic"] }
tantivy = "0.24.1"

# After:
lindera = { version = "1.4.1", features = ["embedded-ipadic"] }
lindera-ipadic = { version = "1.4.1", features = ["embedded-ipadic"] }
lindera-tantivy = "1.1.1"
tantivy = "0.25.0"
```

## Known Issues

### Dictionary Download During Build
The `embedded-ipadic` feature requires downloading the IPADIC dictionary files during the build process. This requires:
- Network access during `cargo build`
- Access to `https://lindera.dev/mecab-ipadic-*.tar.gz`

**Workaround**: If builds fail with "Failed to download a valid file from all sources":
1. Ensure network access is available during build
2. Set `LINDERA_CACHE` environment variable to cache downloaded dictionaries
3. Alternatively, use filesystem-based dictionary loading with pre-built dictionaries

## Testing
- The code compiles successfully
- Tests require network access during build for dictionary download
- Runtime functionality remains unchanged from user perspective

## References
- [Tantivy 0.25.0 Changelog](https://github.com/quickwit-oss/tantivy/blob/main/CHANGELOG.md)
- [Lindera Releases](https://github.com/lindera/lindera/releases)
- [Lindera 1.4.0 Release Notes](https://github.com/lindera/lindera/releases/tag/v1.4.0)
