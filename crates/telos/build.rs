use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const BUILD_HINT: &str = "run `npm ci && npm run build` in frontend/ first";

struct FrontendAsset {
    source: PathBuf,
    path: String,
    content_type: &'static str,
}

fn main() {
    if let Err(error) = build() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn build() -> Result<(), String> {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| "CARGO_MANIFEST_DIR is not set while building telos".to_string())?,
    );
    let dist = manifest_dir.join("../../frontend/dist");
    emit_rerun_if_changed(&dist)?;
    println!("cargo::rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let mut assets = collect_assets(&dist)?;
    assets.sort_by(|left, right| left.path.cmp(&right.path));
    for pair in assets.windows(2) {
        if pair[0].path == pair[1].path {
            return Err(format!(
                "duplicate frontend asset path `{}` in {}",
                pair[0].path,
                dist.display()
            ));
        }
    }
    if !assets.iter().any(|asset| asset.path == "index.html") {
        return Err(invalid_dist(&dist, "index.html is missing"));
    }

    for asset in &assets {
        emit_rerun_if_changed(&asset.source)?;
    }

    let output = PathBuf::from(
        env::var_os("OUT_DIR")
            .ok_or_else(|| "OUT_DIR is not set while building telos".to_string())?,
    )
    .join("frontend_assets.rs");
    fs::write(&output, generated_assets(&assets))
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;

    let build_date = build_date(&manifest_dir)?;
    for dependency in &build_date.git_dependencies {
        emit_rerun_if_changed(dependency)?;
    }
    println!("cargo::rustc-env=TELOS_BUILD_DATE={}", build_date.date);
    Ok(())
}

fn collect_assets(dist: &Path) -> Result<Vec<FrontendAsset>, String> {
    let metadata = match fs::symlink_metadata(dist) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid_dist(dist, "directory is missing"));
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect frontend directory {}: {error}",
                dist.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(invalid_dist(dist, "directory must not be a symlink"));
    }
    if !metadata.is_dir() {
        return Err(invalid_dist(dist, "path is not a directory"));
    }

    let mut files = Vec::new();
    visit(dist, dist, &mut files)?;
    if files.is_empty() {
        return Err(invalid_dist(dist, "directory contains no regular files"));
    }
    Ok(files)
}

fn visit(root: &Path, directory: &Path, assets: &mut Vec<FrontendAsset>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to read frontend directory {}: {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read an entry in {}: {error}",
                directory.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect frontend asset {}: {error}",
                entry.path().display()
            )
        })?;
        if file_type.is_dir() {
            visit(root, &entry.path(), assets)?;
        } else if file_type.is_file() {
            assets.push(validate_asset(root, &entry.path())?);
        }
    }
    Ok(())
}

fn validate_asset(root: &Path, source: &Path) -> Result<FrontendAsset, String> {
    let relative = source.strip_prefix(root).map_err(|error| {
        format!(
            "frontend asset {} is outside {}: {error}",
            source.display(),
            root.display()
        )
    })?;
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    format!("frontend asset path is not UTF-8: {}", source.display())
                })?;
                if matches!(value, "." | "..") || value.contains('\\') {
                    return Err(format!(
                        "frontend asset has a dangerous path component: {}",
                        source.display()
                    ));
                }
                if value.chars().any(char::is_control) {
                    return Err(format!(
                        "frontend asset path contains a control character: {source:?}"
                    ));
                }
                components.push(value);
            }
            _ => {
                return Err(format!(
                    "frontend asset has a dangerous path component: {}",
                    source.display()
                ));
            }
        }
    }
    if components.is_empty() || components.iter().any(|component| component.is_empty()) {
        return Err(format!(
            "frontend asset has an invalid path: {}",
            source.display()
        ));
    }
    let file_name = components[components.len() - 1];
    if matches!(file_name, "data.js" | ".nojekyll") {
        return Err(format!(
            "frontend asset uses reserved name `{file_name}`: {}",
            source.display()
        ));
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "frontend asset has no allowed extension: {}",
                source.display()
            )
        })?;
    let content_type = content_type(extension).ok_or_else(|| {
        format!(
            "frontend asset extension `.{extension}` is not allowed: {}",
            source.display()
        )
    })?;

    Ok(FrontendAsset {
        source: source.to_path_buf(),
        path: components.join("/"),
        content_type,
    })
}

fn content_type(extension: &str) -> Option<&'static str> {
    match extension {
        "html" => Some("text/html; charset=utf-8"),
        "js" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "ico" => Some("image/x-icon"),
        "txt" => Some("text/plain; charset=utf-8"),
        "woff2" => Some("font/woff2"),
        "webmanifest" => Some("application/manifest+json"),
        _ => None,
    }
}

fn generated_assets(assets: &[FrontendAsset]) -> String {
    let mut generated = String::from("pub(crate) static ASSETS: &[Asset] = &[\n");
    for asset in assets {
        generated.push_str(&format!(
            "    Asset {{ path: {:?}, content_type: {:?}, bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../frontend/dist/\", {:?})) }},\n",
            asset.path, asset.content_type, asset.path
        ));
    }
    generated.push_str("];\n");
    generated
}

fn invalid_dist(dist: &Path, reason: &str) -> String {
    format!(
        "invalid frontend bundle at {} ({reason}); {BUILD_HINT}",
        dist.display()
    )
}

struct BuildDate {
    date: String,
    git_dependencies: Vec<PathBuf>,
}

fn build_date(manifest_dir: &Path) -> Result<BuildDate, String> {
    let (seconds, git_dependencies) = match env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => (
            value
                .parse::<i64>()
                .map_err(|error| format!("invalid SOURCE_DATE_EPOCH value `{value}`: {error}"))?,
            Vec::new(),
        ),
        Err(env::VarError::NotPresent) => match git_timestamp(manifest_dir) {
            Some(seconds) => (seconds, git_dependencies(manifest_dir)?),
            None => (current_timestamp(), Vec::new()),
        },
        Err(env::VarError::NotUnicode(_)) => {
            return Err("SOURCE_DATE_EPOCH is not valid UTF-8".to_string());
        }
    };
    Ok(BuildDate {
        date: date_from_unix_seconds(seconds)?,
        git_dependencies,
    })
}

fn emit_rerun_if_changed(path: &Path) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("cannot emit a Cargo rerun path that is not UTF-8: {path:?}"))?;
    if path.chars().any(char::is_control) {
        return Err(format!(
            "cannot emit a Cargo rerun path containing a control character: {path:?}"
        ));
    }
    println!("cargo::rerun-if-changed={path}");
    Ok(())
}

fn git_dependencies(manifest_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut names = vec!["HEAD".to_string(), "logs/HEAD".to_string()];
    if let Some(reference) = git_output(manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        names.push(reference.clone());
        names.push(format!("logs/{reference}"));
    }
    names.push("packed-refs".to_string());

    let paths = names
        .iter()
        .map(|name| {
            git_output(
                manifest_dir,
                &["rev-parse", "--path-format=absolute", "--git-path", name],
            )
            .map(PathBuf::from)
            .ok_or_else(|| format!("failed to resolve Git metadata path `{name}`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths = paths
        .into_iter()
        .filter_map(|path| match fs::symlink_metadata(&path) {
            Ok(_) => Some(Ok(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => Some(Err(format!(
                "failed to inspect Git metadata path {path:?}: {error}"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn git_output(manifest_dir: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = std::str::from_utf8(&output.stdout).ok()?.trim();
    if output.is_empty() || output.contains(['\r', '\n']) {
        return None;
    }
    Some(output.to_string())
}

fn git_timestamp(manifest_dir: &Path) -> Option<i64> {
    git_output(manifest_dir, &["log", "-1", "--format=%ct"])?
        .parse()
        .ok()
}

fn current_timestamp() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

fn date_from_unix_seconds(seconds: i64) -> Result<String, String> {
    let days = seconds.div_euclid(86_400);
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if !(0..=9_999).contains(&year) {
        return Err(format!(
            "Unix timestamp {seconds} resolves to unsupported UTC year {year}; supported date range is 0000-01-01 through 9999-12-31"
        ));
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}
