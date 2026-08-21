#![cfg_attr(not(test), allow(dead_code))]

pub(crate) struct Asset {
    pub(crate) path: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/frontend_assets.rs"));

/// Looks up a bundle path with or without one leading slash.
pub(crate) fn lookup(path: &str) -> Option<&'static Asset> {
    let path = path.strip_prefix('/').unwrap_or(path);
    ASSETS
        .binary_search_by_key(&path, |asset| asset.path)
        .ok()
        .map(|index| &ASSETS[index])
}

#[cfg(test)]
mod tests {
    use super::{ASSETS, lookup};

    #[test]
    fn embedded_bundle_contains_a_non_empty_index() {
        let index = lookup("/index.html").expect("index.html is embedded");

        assert!(!index.bytes.is_empty());
        assert_eq!(index.content_type, "text/html; charset=utf-8");
    }

    #[test]
    fn embedded_paths_are_sorted_unique_and_safe() {
        let paths = ASSETS.iter().map(|asset| asset.path).collect::<Vec<_>>();
        let mut expected = paths.clone();
        expected.sort_unstable();
        expected.dedup();

        assert_eq!(paths, expected);
        assert!(paths.iter().all(|path| {
            !path.starts_with('/')
                && !path.split('/').any(|component| component == "..")
                && !path.contains('\\')
        }));
    }

    #[test]
    fn lookup_accepts_one_optional_leading_slash_and_rejects_unknown_paths() {
        let relative = lookup("assets/app.js").expect("relative asset path is accepted");
        let absolute = lookup("/assets/app.js").expect("one leading slash is accepted");

        assert!(std::ptr::eq(relative, absolute));
        assert_eq!(relative.content_type, "text/javascript; charset=utf-8");
        assert_eq!(
            lookup("/assets/app.css").unwrap().content_type,
            "text/css; charset=utf-8"
        );
        assert_eq!(
            lookup("/assets/logo.png").unwrap().content_type,
            "image/png"
        );
        assert!(lookup("/missing.js").is_none());
        assert!(lookup("//assets/app.js").is_none());
    }
}
