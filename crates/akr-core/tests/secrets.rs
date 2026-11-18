//! Private data is absent from the public compiler inputs, not merely hidden by a UI.

use akr_core::{json::Value, ops::Staged, resolve::load_workspace, secrets};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "akr-secret-core-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join(".akr")).unwrap();
        std::fs::write(
            path.join(".akr/project.akr"),
            "akr 0.1\nproject private-test\nnamespace private-test \"Test.\"\n",
        )
        .unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opaque_immutable_entries_roundtrip_without_value_derived_identity() {
    let root = Root::new();
    let value = "D:\\developer\\private-key.txt\nไทย 🦀\n\"quoted\"\n";
    let first = secrets::put(&root.0, value).unwrap();
    std::fs::write(root.0.join(".akr/secrets/.gitignore"), "!*.json\n").unwrap();
    let second = secrets::put(&root.0, value).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.0.join(".akr/secrets/.gitignore")).unwrap(),
        "*\n"
    );
    assert_ne!(first, second);
    assert_eq!(first.len(), "{{secret:}}".len() + 32);
    assert!(!first.contains("developer"));
    assert_eq!(
        secrets::get(&root.0, &first).unwrap().as_deref(),
        Some(value)
    );
    assert_eq!(
        secrets::resolve(&root.0, &format!("Config {first}; {second}."), false).unwrap(),
        "Config [*]; [*]."
    );
    assert_eq!(secrets::resolve(&root.0, &first, true).unwrap(), value);
    assert_eq!(
        akr_core::render::slug(&format!("Config {first}")),
        akr_core::render::slug("Config [*]")
    );
    assert!(secrets::get(&root.0, "../../outside").is_err());
    assert!(secrets::get(&root.0, "{{secret:not-an-id}}").is_err());
}

#[test]
fn public_outputs_do_not_read_storage_and_missing_local_values_are_normal() {
    let root = Root::new();
    let reference = secrets::put(&root.0, "synthetic-private-marker").unwrap();
    let nested = secrets::put(&root.0, &reference).unwrap();
    assert_eq!(
        secrets::resolve(&root.0, &nested, true).unwrap(),
        reference,
        "inserted text must not be recursively resolved"
    );
    let mut json = Value::object(vec![(
        "nested",
        Value::array(vec![Value::string(&reference)]),
    )]);
    secrets::redact_json(&mut json);
    assert!(!json.to_pretty().contains("secret:"));
    std::fs::remove_dir_all(root.0.join(secrets::DIRECTORY)).unwrap();
    assert_eq!(secrets::resolve(&root.0, &reference, true).unwrap(), "[*]");
    assert_eq!(
        secrets::redact(&format!("🦀{reference}{reference}")),
        "🦀[*][*]"
    );
    assert_eq!(
        secrets::redact("ordinary {{secret:not-a-citation}}"),
        "ordinary {{secret:not-a-citation}}"
    );
}

#[test]
fn compilers_never_walk_private_storage_even_for_akr_files() {
    let root = Root::new();
    let before = load_workspace(&root.0, &root.0.join(".akr")).unwrap();
    secrets::put(&root.0, "synthetic-private-marker").unwrap();
    std::fs::write(
        root.0.join(".akr/secrets/invalid.akr"),
        "PRIVATE DATA THAT IS NOT AKR",
    )
    .unwrap();
    let after = load_workspace(&root.0, &root.0.join(".akr")).unwrap();
    assert_eq!(before.inputs.sources.len(), after.inputs.sources.len());
    assert_eq!(before.inputs.sources[0].hash, after.inputs.sources[0].hash);
    assert!(after.diagnostics.is_empty());
    let staged = Staged::load(&root.0.join(".akr")).unwrap();
    assert!(staged.diagnostics.is_empty());
    assert!(staged.texts.keys().all(|p| !p.starts_with("secrets")));
    // Only the reserved top-level category is excluded, not a public namespace.
    std::fs::create_dir_all(root.0.join(".akr/records/secrets")).unwrap();
    std::fs::write(
        root.0.join(".akr/records/secrets/invalid.akr"),
        "INVALID PUBLIC RECORD",
    )
    .unwrap();
    assert!(
        !load_workspace(&root.0, &root.0.join(".akr"))
            .unwrap()
            .diagnostics
            .is_empty()
    );
    assert!(
        !Staged::load(&root.0.join(".akr"))
            .unwrap()
            .diagnostics
            .is_empty()
    );
}

#[test]
fn corrupt_entry_errors_do_not_echo_private_bytes_and_public_redaction_still_works() {
    let root = Root::new();
    let reference = secrets::put(&root.0, "synthetic-private-marker").unwrap();
    let opaque = &reference[9..41];
    std::fs::write(
        root.0.join(format!(".akr/secrets/{opaque}.json")),
        "PRIVATE malformed JSON",
    )
    .unwrap();
    let error = secrets::get(&root.0, &reference).unwrap_err().to_string();
    assert!(!error.contains("PRIVATE"));
    assert_eq!(secrets::resolve(&root.0, &reference, false).unwrap(), "[*]");
    assert!(
        secrets::require_public_input(
            &root.0,
            &PathBuf::from(format!(".akr/secrets/../secrets/{opaque}.json"))
        )
        .is_err()
    );
    assert!(secrets::is_private_path(".AKR\\SECRETS\\entry.json"));
    assert!(secrets::is_private_path(
        "nested/project/.akr/secrets/entry.json"
    ));
    assert!(!secrets::is_private_path(".akr/secrets-guide.md"));
}

#[cfg(unix)]
#[test]
fn permissions_are_private_and_storage_links_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = Root::new();
    let reference = secrets::put(&root.0, "synthetic-private-marker").unwrap();
    let path = root
        .0
        .join(format!(".akr/secrets/{}.json", &reference[9..41]));
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(root.0.join(secrets::DIRECTORY))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::write(root.0.join("outside"), "PRIVATE").unwrap();
    symlink(root.0.join("outside"), &path).unwrap();
    assert!(secrets::get(&root.0, &reference).is_err());
    assert!(secrets::require_public_input(&root.0, &path).is_err());
    std::fs::remove_dir_all(root.0.join(secrets::DIRECTORY)).unwrap();
    symlink(root.0.join("outside"), root.0.join(secrets::DIRECTORY)).unwrap();
    assert!(secrets::put(&root.0, "PRIVATE").is_err());
}
