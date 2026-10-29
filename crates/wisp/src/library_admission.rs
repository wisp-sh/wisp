//! Read untrusted release files without following links or trusting a stale size.
use anyhow::{Context, Result, ensure};
use std::{fs, io::Read, path::Path};

pub(super) const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_METADATA_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_GO_DESCRIPTOR_BYTES: u64 = 16 * 1024;

pub(super) fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let read = || -> Result<Vec<u8>> {
        // Reject devices before opening them. The flags and handle metadata below
        // also protect against replacement between this check and the open.
        ensure!(
            fs::symlink_metadata(path)?.is_file(),
            "expected a regular file"
        );
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "expected a regular file");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            ensure!(
                metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                "unexpected reparse point"
            );
        }
        ensure!(metadata.len() <= limit, "file exceeds {limit} byte limit");
        read_limited(file, limit)
    };
    read().with_context(|| format!("invalid library release file: {}", path.display()))
}

fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "file exceeds {limit} byte limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_oversized_go_descriptors_before_archive_work() {
        use super::super::{digest, targets, tools, validate};
        use serde_json::json;
        let directory = tempfile::tempdir().unwrap();
        let component = wasm_encoder::Component::new().finish();
        let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
            wit_parser::decoding::decode(&component).unwrap()
        else {
            panic!("expected Component fixture");
        };
        let release = json!({
            "schema": 3,
            "kind": "component-library",
            "identity": "test:admission",
            "version": "0.1.0",
            "componentSha256": digest(&component),
            "contract": crate::wit_model::AbiModel::from_resolve(&resolve, world).unwrap(),
            "packages": {"rust": {}, "node": {}, "go": {
                "name": "example.test/admission", "sha256": "", "platform": crate::library_go::portable::PLATFORM
            }},
            "targets": targets(false, false, true, false),
            "tools": tools(),
            "metadata": crate::library_metadata::Metadata::default(),
            "targetRequirements": {}
        });
        fs::write(directory.path().join("component.wasm"), component).unwrap();
        fs::write(
            directory.path().join("library-release.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
        let descriptors = [
            (
                "go.mod",
                crate::library_go::portable::module_file("example.test/admission"),
            ),
            (
                "go.info",
                crate::library_go::module_metadata("example.test/admission", "0.1.0")
                    .unwrap()
                    .1,
            ),
        ];
        for (name, bytes) in &descriptors {
            fs::write(directory.path().join(name), bytes).unwrap();
        }
        // A valid manifest and Component reach the missing archive. These tests
        // stop before regeneration, so they need no compilers or generators.
        assert!(format!("{:#}", validate(directory.path()).unwrap_err()).contains("go.zip"));
        for (name, bytes) in descriptors {
            let path = directory.path().join(name);
            fs::File::create(&path)
                .unwrap()
                .set_len(MAX_GO_DESCRIPTOR_BYTES + 1)
                .unwrap();
            let error = format!("{:#}", validate(directory.path()).unwrap_err());
            assert!(
                error.contains(name) && error.contains("byte limit"),
                "{error}"
            );
            fs::write(path, bytes).unwrap();
        }
    }

    #[test]
    fn descriptors_accept_exact_limits_and_reject_sparse_oversize_files() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["go.mod", "go.info"] {
            let path = directory.path().join(name);
            let file = fs::File::create(&path).unwrap();
            file.set_len(MAX_GO_DESCRIPTOR_BYTES).unwrap();
            assert_eq!(
                read_file(&path, MAX_GO_DESCRIPTOR_BYTES).unwrap().len() as u64,
                MAX_GO_DESCRIPTOR_BYTES
            );
            file.set_len(MAX_GO_DESCRIPTOR_BYTES + 1).unwrap();
            let error = read_file(&path, MAX_GO_DESCRIPTOR_BYTES).unwrap_err();
            assert!(format!("{error:#}").contains("byte limit"), "{error:#}");
        }
    }

    #[test]
    fn reads_stay_bounded_even_if_a_file_grows_after_the_metadata_check() {
        // An endless reader models a file growing faster than it can be read.
        assert!(read_limited(std::io::repeat(0), MAX_GO_DESCRIPTOR_BYTES).is_err());
        assert_eq!(
            read_limited(
                &b"module example.test/library\n"[..],
                MAX_GO_DESCRIPTOR_BYTES
            )
            .unwrap(),
            b"module example.test/library\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptors_reject_symlinks_directories_sockets_and_fifos_without_waiting() {
        use std::{
            os::unix::{fs::symlink, net::UnixListener},
            sync::mpsc,
            time::Duration,
        };
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        fs::write(&target, b"descriptor").unwrap();
        for name in ["go.mod", "go.info"] {
            let path = directory.path().join(name);
            symlink(&target, &path).unwrap();
            assert!(read_file(&path, MAX_GO_DESCRIPTOR_BYTES).is_err());
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            assert!(read_file(&path, MAX_GO_DESCRIPTOR_BYTES).is_err());
            fs::remove_dir(&path).unwrap();
            let socket = UnixListener::bind(&path).unwrap();
            assert!(read_file(&path, MAX_GO_DESCRIPTOR_BYTES).is_err());
            drop(socket);
            fs::remove_file(&path).unwrap();
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            let (send, receive) = mpsc::channel();
            let reader = std::thread::spawn(move || {
                send.send(read_file(&path, MAX_GO_DESCRIPTOR_BYTES))
                    .unwrap()
            });
            assert!(
                receive
                    .recv_timeout(Duration::from_secs(2))
                    .expect("admission blocked on a FIFO")
                    .is_err()
            );
            reader.join().unwrap();
        }
    }
}
