//! Prepared publication of exact schema-3 archives to independent registries.
use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder, Response},
    header::HeaderValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

const LIMIT: usize = 128 * 1024 * 1024;
const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const FILES: [&str; 4] = [
    "library-release.json",
    "component.wasm",
    "rust.crate",
    "node.tgz",
];
type Files = BTreeMap<String, Vec<u8>>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CargoConfig {
    index: String,
    api: String,
    package: String,
    token_env: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NpmConfig {
    registry: String,
    package: String,
    access: String,
    token_env: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ComponentConfig {
    registry: String,
    repository: String,
    token_env: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GoConfig {
    directory: String,
    module: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    cargo: CargoConfig,
    npm: NpmConfig,
    component: ComponentConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    go: Option<GoConfig>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .with_context(|| format!("missing {name}"))
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && meta.len() <= limit as u64,
        "invalid or oversized file: {}",
        path.display()
    );
    let bytes = fs::read(path)?;
    ensure!(bytes.len() <= limit, "file size limit");
    Ok(bytes)
}

fn endpoint(text: &str, configuration: bool) -> Result<Url> {
    let url = Url::parse(text).context("invalid registry URL")?;
    let local = regex::Regex::new(r"^http://127\.0\.0\.1(?::[0-9]{1,5})?(?:/|$)")?.is_match(text);
    ensure!(
        url.scheme() == "https" || local,
        "registry URL requires HTTPS or literal loopback HTTP"
    );
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && (!configuration || url.query().is_none()),
        "registry URL must not contain credentials, fragments or configuration queries"
    );
    Ok(url)
}

fn config(value: &Value) -> Result<Config> {
    let config: Config = serde_json::from_value(value.clone())?;
    for url in [
        &config.cargo.index,
        &config.cargo.api,
        &config.npm.registry,
        &config.component.registry,
    ] {
        endpoint(url, true)?;
    }
    for name in [
        &config.cargo.token_env,
        &config.npm.token_env,
        &config.component.token_env,
    ]
    .into_iter()
    .flatten()
    {
        ensure!(
            regex::Regex::new(r"^[A-Z_][A-Z0-9_]{0,127}$")?.is_match(name),
            "credential configuration must name an environment variable"
        );
    }
    ensure!(
        regex::Regex::new(r"^[a-z][a-z0-9_-]{0,63}$")?.is_match(&config.cargo.package),
        "invalid Cargo package name"
    );
    ensure!(
        regex::Regex::new(r"^@[a-z][a-z0-9-]*/[a-z][a-z0-9-]*$")?.is_match(&config.npm.package),
        "invalid scoped npm package name"
    );
    ensure!(
        matches!(config.npm.access.as_str(), "public" | "restricted"),
        "npm access must be explicit"
    );
    ensure!(
        config.component.repository.len() <= 240
            && regex::Regex::new(
                r"^[a-z0-9]+(?:[._-][a-z0-9]+)*(?:/[a-z0-9]+(?:[._-][a-z0-9]+)*)*$"
            )?
            .is_match(&config.component.repository),
        "invalid OCI repository"
    );
    if let Some(go) = &config.go {
        ensure!(
            Path::new(&go.directory).is_absolute(),
            "Go proxy directory must be absolute"
        );
        ensure!(
            go.module.len() <= 240
                && regex::Regex::new(
                    r"^[a-z0-9][a-z0-9.-]+\.[a-z0-9.-]+(?:/[a-z0-9][a-z0-9._-]*)+$"
                )?
                .is_match(&go.module)
                && go.module.split('/').all(|part| part != "." && part != ".."),
            "invalid Go module path"
        );
    }
    Ok(config)
}

#[cfg(test)]
pub fn validate_config(value: &Value) -> Result<()> {
    config(value).map(|_| ())
}

fn snapshot(dir: &Path) -> Result<(Value, Files)> {
    let mut files = Files::new();
    let temp = tempfile::tempdir()?;
    let mut total = 0;
    for name in FILES {
        let bytes = read(&dir.join(name), LIMIT)?;
        total += bytes.len();
        ensure!(total <= 2 * LIMIT, "publication size limit");
        fs::write(temp.path().join(name), &bytes)?;
        files.insert(name.into(), bytes);
    }
    let record = crate::release::strict_json(&files["library-release.json"])?;
    if record["packages"].get("go").is_some() {
        for name in ["go.zip", "go.mod", "go.info"] {
            let bytes = read(&dir.join(name), LIMIT)?;
            total += bytes.len();
            ensure!(total <= 3 * LIMIT, "publication size limit");
            fs::write(temp.path().join(name), &bytes)?;
            files.insert(name.into(), bytes);
        }
    }
    // Validate the private snapshot so publication uses precisely the bytes checked.
    crate::library::validate(temp.path())?;
    Ok((
        crate::release::strict_json(&files["library-release.json"])?,
        files,
    ))
}

fn descriptor(media_type: &str, bytes: &[u8]) -> Value {
    json!({"mediaType":media_type,"digest":format!("sha256:{}",hash(bytes)),"size":bytes.len()})
}

fn oci_manifest(record: &Value, files: &Files) -> Value {
    let layers = [
        ("component.wasm", "application/wasm"),
        (
            "library-release.json",
            "application/vnd.wisp.library.release.v3+json",
        ),
        ("rust.crate", "application/vnd.wisp.cargo.crate.v1+gzip"),
        ("node.tgz", "application/vnd.wisp.npm.package.v1+gzip"),
        ("go.zip", "application/vnd.wisp.go.module.v1+zip"),
        ("go.mod", "text/plain"),
        ("go.info", "application/json"),
    ]
    .into_iter()
    .filter(|(name, _)| files.contains_key(*name))
    .map(|(name, media)| {
        let mut value = descriptor(media, &files[name]);
        value["annotations"] = json!({"org.opencontainers.image.title":name});
        value
    })
    .collect::<Vec<_>>();
    json!({"schemaVersion":2,"mediaType":MANIFEST_TYPE,"artifactType":"application/vnd.wisp.component-library.v1",
        "config":descriptor("application/vnd.oci.empty.v1+json", b"{}"),"layers":layers,
        "annotations":{"org.opencontainers.image.title":record["identity"],"org.opencontainers.image.version":record["version"]}})
}

fn plan(dir: &Path, config: &Config, record: &Value, files: &Files) -> Result<Value> {
    ensure!(
        record["packages"]["rust"]["name"] == config.cargo.package
            && record["packages"]["node"]["name"] == config.npm.package,
        "configured package names differ from admitted library packages"
    );
    ensure!(
        config.go.is_some() == record["packages"].get("go").is_some(),
        "Go publication configuration must match the release artifacts"
    );
    if let Some(go) = &config.go {
        ensure!(
            record["packages"]["go"]["name"] == go.module,
            "configured Go module differs from admitted package"
        );
    }
    let manifest = oci_manifest(record, files);
    let digest = format!("sha256:{}", hash(&serde_json::to_vec(&manifest)?));
    let artifacts = files
        .iter()
        .map(|(name, bytes)| {
            (
                name.clone(),
                json!({"sha256":hash(bytes),"size":bytes.len()}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    Ok(
        json!({"schema":1,"kind":"wisp-library-publication","releaseDirectory":dir.canonicalize()?,
        "identity":record["identity"],"version":record["version"],"registries":config,"artifacts":artifacts,
        "component":{"manifest":manifest,"digest":digest,"reference":format!("{}/{}@{}",config.component.registry.trim_end_matches('/'),config.component.repository,digest)}}),
    )
}

pub fn prepare(dir: &Path, configuration: &Path, output: &Path) -> Result<Value> {
    let config = config(&crate::release::strict_json(&read(
        configuration,
        1024 * 1024,
    )?)?)?;
    let (record, files) = snapshot(dir)?;
    let plan = plan(dir, &config, &record, &files)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .context("choose an absent publication plan path")?;
    file.write_all(&serde_json::to_vec_pretty(&plan)?)?;
    file.sync_all()?;
    Ok(plan)
}

/// Retrieve an immutable OCI release and admit it before making it available
/// to the directory-backed Component resolver.
pub fn fetch(configuration: &Path, digest: &str, output: &Path) -> Result<Value> {
    ensure!(!output.try_exists()?, "fetch output already exists");
    ensure!(
        regex::Regex::new(r"^sha256:[0-9a-f]{64}$")?.is_match(digest),
        "fetch requires an immutable sha256 digest"
    );
    let config = config(&crate::release::strict_json(&read(
        configuration,
        1024 * 1024,
    )?)?)?;
    let http = Http::new()?;
    let auth = credential(&config.component.token_env, true)?;
    let base = format!(
        "{}/v2/{}",
        config.component.registry.trim_end_matches('/'),
        config.component.repository
    );
    let bytes = http
        .get(
            &format!("{base}/manifests/{digest}"),
            auth.as_ref(),
            1024 * 1024,
        )?
        .context("Component release not found")?;
    ensure!(
        format!("sha256:{}", hash(&bytes)) == digest,
        "OCI manifest digest mismatch"
    );
    let manifest = crate::release::strict_json(&bytes)?;
    let layers = manifest["layers"]
        .as_array()
        .context("OCI release layers")?;
    ensure!(
        matches!(layers.len(), 4 | 7),
        "unexpected OCI release layers"
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    let mut files = Files::new();
    let mut total = 0;
    for layer in layers {
        let name = layer["annotations"]["org.opencontainers.image.title"]
            .as_str()
            .context("OCI layer title")?;
        ensure!(
            FILES.contains(&name) || ["go.zip", "go.mod", "go.info"].contains(&name),
            "unrecognized OCI layer"
        );
        ensure!(!files.contains_key(name), "duplicate OCI layer");
        let digest = field(layer, "digest")?;
        ensure!(
            regex::Regex::new(r"^sha256:[0-9a-f]{64}$")?.is_match(digest),
            "invalid OCI layer digest"
        );
        let size = usize::try_from(layer["size"].as_u64().context("OCI layer size")?)?;
        ensure!(
            size <= LIMIT && total + size <= 3 * LIMIT,
            "OCI release size limit"
        );
        total += size;
        let bytes = http
            .get(&format!("{base}/blobs/{digest}"), auth.as_ref(), size)?
            .context("OCI layer missing")?;
        ensure!(
            bytes.len() == size && format!("sha256:{}", hash(&bytes)) == digest,
            "OCI layer digest mismatch"
        );
        fs::write(stage.path().join(name), &bytes)?;
        files.insert(name.into(), bytes);
    }
    let admitted = crate::library::validate(stage.path())?;
    let record = crate::release::strict_json(
        files
            .get("library-release.json")
            .context("release manifest missing")?,
    )?;
    ensure!(
        manifest == oci_manifest(&record, &files),
        "OCI release manifest differs from admitted artifacts"
    );
    ensure!(
        record["packages"]["rust"]["name"] == config.cargo.package
            && record["packages"]["node"]["name"] == config.npm.package,
        "downloaded package names differ from configuration"
    );
    let hosts = record["contract"]["imports"]
        .as_array()
        .context("Component imports")?
        .iter()
        .map(|interface| {
            let package = field(interface, "package")?;
            let (package, version) = package.rsplit_once('@').context("versioned host import")?;
            Ok(format!("{package}/{}@{version}", field(interface, "name")?))
        })
        .collect::<Result<Vec<_>>>()?;
    let dependency = json!({"schema":1,"kind":"component-dependency-package","identity":record["identity"],"version":record["version"],
        "component":{"path":"component.wasm","sha256":record["componentSha256"]},"dependencies":{},"hostImports":hosts});
    fs::write(
        stage.path().join("component.json"),
        serde_json::to_vec_pretty(&dependency)?,
    )?;
    fs::create_dir(output)?;
    if let Err(error) = fs::rename(stage.path(), output) {
        let _ = fs::remove_dir(output);
        return Err(error.into());
    }
    Ok(
        json!({"admitted":admitted,"digest":digest,"directory":output.canonicalize()?,"componentManifest":"component.json"}),
    )
}

/// Cargo's registry upload framing. Dependency registry identities are explicit.
pub fn cargo_upload_body(manifest: &toml::Value, archive: &[u8]) -> Result<Vec<u8>> {
    let mut deps = Vec::new();
    let mut add = |table: &toml::Value, target: Option<&str>| -> Result<()> {
        for (section, kind) in [
            ("dependencies", "normal"),
            ("build-dependencies", "build"),
            ("dev-dependencies", "dev"),
        ] {
            if let Some(values) = table.get(section).and_then(toml::Value::as_table) {
                for (alias, value) in values {
                    let version = value
                        .as_str()
                        .or_else(|| value.get("version").and_then(toml::Value::as_str))
                        .context("dependency version")?;
                    let original = value.get("package").and_then(toml::Value::as_str);
                    deps.push(json!({"name":original.unwrap_or(alias),"version_req":version,"features":value.get("features").map(serde_json::to_value).transpose()?.unwrap_or(json!([])),
                        "optional":value.get("optional").and_then(toml::Value::as_bool).unwrap_or(false),
                        "default_features":value.get("default-features").and_then(toml::Value::as_bool).unwrap_or(true),
                        "target":target,"kind":kind,"registry":"https://github.com/rust-lang/crates.io-index","explicit_name_in_toml":original.map(|_| alias)}));
                }
            }
        }
        Ok(())
    };
    add(manifest, None)?;
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, table) in targets {
            add(table, Some(target))?;
        }
    }
    let package = manifest.get("package").context("Cargo package metadata")?;
    let mut metadata = json!({"name":package.get("name").and_then(toml::Value::as_str).context("Cargo package name")?,
        "vers":package.get("version").and_then(toml::Value::as_str).context("Cargo package version")?,"deps":deps,"features":{},"authors":[],"keywords":[],"categories":[],"badges":{}});
    for (source, destination) in [
        ("description", "description"),
        ("documentation", "documentation"),
        ("homepage", "homepage"),
        ("license", "license"),
        ("license-file", "license_file"),
        ("repository", "repository"),
        ("links", "links"),
        ("rust-version", "rust_version"),
    ] {
        metadata[destination] = package
            .get(source)
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or(Value::Null);
    }
    if let Some(features) = manifest.get("features") {
        metadata["features"] = serde_json::to_value(features)?;
    }
    for key in ["authors", "keywords", "categories"] {
        if let Some(value) = package.get(key) {
            metadata[key] = serde_json::to_value(value)?;
        }
    }
    let json = serde_json::to_vec(&metadata)?;
    let mut body = u32::try_from(json.len())?.to_le_bytes().to_vec();
    body.extend(json);
    body.extend(u32::try_from(archive.len())?.to_le_bytes());
    body.extend_from_slice(archive);
    Ok(body)
}

pub fn npm_publish_document(
    manifest: &Value,
    archive: &[u8],
    registry: &str,
    access: &str,
) -> Result<Value> {
    let name = field(manifest, "name")?;
    let version = field(manifest, "version")?;
    let filename = format!("{name}-{version}.tgz");
    let mut published = manifest.clone();
    published["_id"] = format!("{name}@{version}").into();
    published["dist"] = json!({"tarball":format!("{}/{name}/-/{filename}",registry.trim_end_matches('/')),
        "shasum":format!("{:x}",sha1::Sha1::digest(archive)),"integrity":format!("sha512-{}",STANDARD.encode(Sha512::digest(archive)))});
    Ok(
        json!({"_id":name,"name":name,"access":access,"dist-tags":{"latest":version},"versions":{version:published},
        "_attachments":{filename:{"content_type":"application/octet-stream","data":STANDARD.encode(archive),"length":archive.len()}}}),
    )
}

struct Http {
    client: Client,
}
impl Http {
    fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(120))
                .user_agent("wisp-library-publisher/1")
                .build()?,
        })
    }
    fn send(&self, request: RequestBuilder) -> Result<Response> {
        request
            .send()
            .map_err(|error| anyhow!("registry request failed: {}", error.without_url()))
    }
    fn get(&self, url: &str, auth: Option<&HeaderValue>, limit: usize) -> Result<Option<Vec<u8>>> {
        endpoint(url, false)?;
        let mut request = self.client.get(url).header(
            "Accept",
            format!("application/json, {MANIFEST_TYPE}, application/octet-stream"),
        );
        if let Some(auth) = auth {
            request = request.header("Authorization", auth);
        }
        let response = self.send(request)?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        ensure!(
            response.status().as_u16() == 200,
            "registry readback failed: HTTP {}",
            response.status()
        );
        Ok(Some(response_bytes(response, limit)?))
    }
    fn verify(&self, url: &str, auth: Option<&HeaderValue>, expected: &[u8]) -> Result<bool> {
        let Some(actual) = self.get(url, auth, expected.len())? else {
            return Ok(false);
        };
        ensure!(
            actual == expected,
            "registry content conflict: existing bytes differ from publication plan"
        );
        Ok(true)
    }
}
fn response_bytes(response: Response, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    response.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "registry response size limit");
    Ok(bytes)
}
fn credential(name: &Option<String>, bearer: bool) -> Result<Option<HeaderValue>> {
    name.as_ref()
        .map(|name| {
            let value = std::env::var(name)
                .with_context(|| format!("set credential environment variable {name}"))?;
            ensure!(!value.is_empty(), "empty registry credential");
            let mut header = HeaderValue::from_str(&if bearer {
                format!("Bearer {value}")
            } else {
                value
            })
            .context("invalid registry credential header")?;
            header.set_sensitive(true);
            Ok(header)
        })
        .transpose()
}
fn same_origin(left: &str, right: &str) -> Result<bool> {
    Ok(endpoint(left, false)?.origin() == endpoint(right, false)?.origin())
}
fn add_auth(request: RequestBuilder, auth: Option<&HeaderValue>) -> RequestBuilder {
    if let Some(auth) = auth {
        request.header("Authorization", auth)
    } else {
        request
    }
}
fn index_path(name: &str) -> String {
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

fn cargo_publish(
    http: &Http,
    config: &CargoConfig,
    record: &Value,
    files: &Files,
    completed: bool,
) -> Result<Value> {
    let auth = credential(&config.token_env, false)?;
    let version = field(record, "version")?;
    let archive = &files["rust.crate"];
    let members = crate::release::archive_files(archive)?;
    let path = format!("{}-{version}/Cargo.toml", config.package);
    let manifest: toml::Value = toml::from_str(std::str::from_utf8(
        members.get(&path).context("Cargo descriptor missing")?,
    )?)?;
    let upload = cargo_upload_body(&manifest, archive)?;
    let metadata_size = u32::from_le_bytes(upload[..4].try_into()?) as usize;
    let uploaded_metadata = crate::release::strict_json(&upload[4..4 + metadata_size])?;
    let index = format!(
        "{}/{}",
        config.index.trim_end_matches('/'),
        index_path(&config.package)
    );
    let configuration = http
        .get(
            &format!("{}/config.json", config.index.trim_end_matches('/')),
            auth.as_ref(),
            1024 * 1024,
        )?
        .context("Cargo registry config missing")?;
    let configuration = crate::release::strict_json(&configuration)?;
    ensure!(
        field(&configuration, "api")?.trim_end_matches('/') == config.api.trim_end_matches('/'),
        "Cargo index API differs from reviewed publication target"
    );
    let verify = || -> Result<bool> {
        let Some(bytes) = http.get(&index, auth.as_ref(), 8 * 1024 * 1024)? else {
            return Ok(false);
        };
        let mut matching = Vec::new();
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let row = crate::release::strict_json(line)?;
            if row["vers"] == version {
                matching.push(row);
            }
        }
        ensure!(
            matching.len() <= 1,
            "duplicate Cargo version in registry index"
        );
        let Some(row) = matching.first() else {
            return Ok(false);
        };
        ensure!(
            row["name"] == config.package && row["cksum"] == hash(archive) && row["yanked"] != true,
            "Cargo version conflict or yanked release"
        );
        let mut expected_dependencies = uploaded_metadata["deps"].as_array().context("Cargo upload dependencies")?.iter().map(|dependency| {
            let renamed = dependency["explicit_name_in_toml"].as_str();
            json!({"name":renamed.unwrap_or(dependency["name"].as_str().unwrap()),"req":dependency["version_req"],
                "features":dependency["features"],"optional":dependency["optional"],"default_features":dependency["default_features"],
                "target":dependency["target"],"kind":dependency["kind"],"registry":dependency["registry"],
                "package":renamed.map(|_| &dependency["name"])})
        }).collect::<Vec<_>>();
        let mut actual_dependencies = row["deps"].as_array().context("Cargo index dependencies missing")?.iter().map(|dependency| {
            let mut normalized = json!({"name":dependency["name"],"req":dependency["req"],"features":dependency.get("features").unwrap_or(&json!([])),
                "optional":dependency["optional"].as_bool().unwrap_or(false),"default_features":dependency["default_features"].as_bool().unwrap_or(true),
                "target":dependency["target"],"kind":dependency["kind"].as_str().unwrap_or("normal"),"registry":dependency["registry"],"package":dependency["package"]});
            if normalized["registry"].is_null() && config.index.trim_end_matches('/') == "https://index.crates.io" {
                normalized["registry"] = "https://github.com/rust-lang/crates.io-index".into();
            }
            normalized
        }).collect::<Vec<_>>();
        expected_dependencies.sort_by_key(Value::to_string);
        actual_dependencies.sort_by_key(Value::to_string);
        ensure!(
            actual_dependencies == expected_dependencies
                && row["features"] == uploaded_metadata["features"],
            "Cargo index dependency metadata conflicts with reviewed archive"
        );
        let mut download = field(&configuration, "dl")?.to_owned();
        if download.contains('{') {
            let path = index_path(&config.package);
            let prefix = path.rsplit_once('/').context("Cargo index prefix")?.0;
            for (marker, value) in [
                ("{crate}", config.package.as_str()),
                ("{version}", version),
                ("{prefix}", prefix),
                ("{lowerprefix}", prefix),
                ("{sha256-checksum}", row["cksum"].as_str().unwrap()),
            ] {
                download = download.replace(marker, value);
            }
            ensure!(
                !download.contains('{'),
                "unsupported Cargo download template"
            );
        } else {
            download = format!(
                "{}/{}/{version}/download",
                download.trim_end_matches('/'),
                config.package
            );
        }
        let download_auth = if same_origin(&download, &config.api)? {
            auth.as_ref()
        } else {
            None
        };
        ensure!(
            http.verify(&download, download_auth, archive)?,
            "Cargo archive missing after index publication"
        );
        Ok(true)
    };
    if !verify()? {
        ensure!(!completed, "previously published Cargo release disappeared");
        let request = http
            .client
            .put(format!(
                "{}/api/v1/crates/new",
                config.api.trim_end_matches('/')
            ))
            .header("Content-Type", "application/octet-stream")
            .header("Accept", "application/json")
            .body(upload.clone());
        let response = http.send(add_auth(request, auth.as_ref()))?;
        let status = response.status();
        ensure!(
            status.is_success() || status.as_u16() == 409,
            "Cargo publication failed: HTTP {status}"
        );
        let body = response_bytes(response, 1024 * 1024)?;
        if !body.is_empty() && status.is_success() {
            ensure!(
                crate::release::strict_json(&body)?.get("errors").is_none(),
                "Cargo registry reported publication errors"
            );
        }
        ensure!(
            verify()?,
            "Cargo index has not exposed this version yet; retry the same plan"
        );
    }
    Ok(json!({"verified":true,"sha256":hash(archive),"package":config.package,"version":version}))
}

fn npm_publish(
    http: &Http,
    config: &NpmConfig,
    record: &Value,
    files: &Files,
    completed: bool,
) -> Result<Value> {
    let auth = credential(&config.token_env, true)?;
    let version = field(record, "version")?;
    let archive = &files["node.tgz"];
    let members = crate::release::archive_files(archive)?;
    let manifest = crate::release::strict_json(
        members
            .get("package/package.json")
            .context("npm descriptor missing")?,
    )?;
    let url = format!(
        "{}/{}",
        config.registry.trim_end_matches('/'),
        config.package.replace('/', "%2f")
    );
    let verify = || -> Result<bool> {
        let Some(bytes) = http.get(&url, auth.as_ref(), 8 * 1024 * 1024)? else {
            return Ok(false);
        };
        let metadata = crate::release::strict_json(&bytes)?;
        let Some(published) = metadata["versions"].get(version) else {
            return Ok(false);
        };
        ensure!(
            published["name"] == config.package && published["version"] == version,
            "npm publication identity conflict"
        );
        for key in [
            "type",
            "main",
            "module",
            "types",
            "exports",
            "imports",
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
            "bundledDependencies",
            "bin",
            "scripts",
        ] {
            ensure!(
                published[key] == manifest[key],
                "npm executable/dependency metadata conflict: {key}"
            );
        }
        ensure!(
            published["dist"]["shasum"] == format!("{:x}", sha1::Sha1::digest(archive))
                && published["dist"]["integrity"]
                    == format!("sha512-{}", STANDARD.encode(Sha512::digest(archive))),
            "npm integrity metadata conflict"
        );
        let download = field(&published["dist"], "tarball")?;
        let download_auth = if same_origin(download, &config.registry)? {
            auth.as_ref()
        } else {
            None
        };
        ensure!(
            http.verify(download, download_auth, archive)?,
            "npm archive missing after publication"
        );
        Ok(true)
    };
    if !verify()? {
        ensure!(!completed, "previously published npm release disappeared");
        let document = npm_publish_document(&manifest, archive, &config.registry, &config.access)?;
        let request = http.client.put(&url).json(&document);
        let response = http.send(add_auth(request, auth.as_ref()))?;
        ensure!(
            response.status().is_success() || response.status().as_u16() == 409,
            "npm publication failed: HTTP {}",
            response.status()
        );
        ensure!(
            verify()?,
            "npm registry has not exposed this version yet; retry the same plan"
        );
    }
    Ok(json!({"verified":true,"sha256":hash(archive),"package":config.package,"version":version}))
}

fn component_publish(
    http: &Http,
    config: &ComponentConfig,
    manifest: &Value,
    files: &Files,
    completed: bool,
) -> Result<Value> {
    let auth = credential(&config.token_env, true)?;
    let base = format!(
        "{}/v2/{}",
        config.registry.trim_end_matches('/'),
        config.repository
    );
    let manifest_bytes = serde_json::to_vec(manifest)?;
    let digest = format!("sha256:{}", hash(&manifest_bytes));
    let url = format!("{base}/manifests/{digest}");
    let existing = http.verify(&url, auth.as_ref(), &manifest_bytes)?;
    ensure!(
        existing || !completed,
        "previously published OCI manifest disappeared"
    );
    for bytes in std::iter::once(b"{}".as_slice()).chain(files.values().map(Vec::as_slice)) {
        let blob_digest = format!("sha256:{}", hash(bytes));
        let blob_url = format!("{base}/blobs/{blob_digest}");
        if !http.verify(&blob_url, auth.as_ref(), bytes)? {
            ensure!(
                !existing,
                "published OCI manifest references a missing blob"
            );
            let request = http
                .client
                .post(format!("{base}/blobs/uploads/"))
                .header("Content-Length", 0);
            let response = http.send(add_auth(request, auth.as_ref()))?;
            ensure!(
                response.status().as_u16() == 202,
                "OCI upload session failed: HTTP {}",
                response.status()
            );
            let location = response
                .headers()
                .get("Location")
                .context("OCI upload Location missing")?
                .to_str()?;
            let mut upload = response.url().join(location)?;
            ensure!(
                same_origin(upload.as_str(), &config.registry)?,
                "OCI upload Location changed origin"
            );
            endpoint(upload.as_str(), false)?;
            upload.query_pairs_mut().append_pair("digest", &blob_digest);
            let request = http
                .client
                .put(upload)
                .header("Content-Type", "application/octet-stream")
                .body(bytes.to_vec());
            let response = http.send(add_auth(request, auth.as_ref()))?;
            ensure!(
                response.status().as_u16() == 201,
                "OCI blob upload failed: HTTP {}",
                response.status()
            );
            ensure!(
                http.verify(&blob_url, auth.as_ref(), bytes)?,
                "OCI blob readback missing"
            );
        }
    }
    if !existing {
        let request = http
            .client
            .put(&url)
            .header("Content-Type", MANIFEST_TYPE)
            .body(manifest_bytes.clone());
        let response = http.send(add_auth(request, auth.as_ref()))?;
        ensure!(
            response.status().as_u16() == 201 || response.status().as_u16() == 409,
            "OCI manifest publication failed: HTTP {}",
            response.status()
        );
        ensure!(
            http.verify(&url, auth.as_ref(), &manifest_bytes)?,
            "OCI manifest readback missing"
        );
    }
    Ok(
        json!({"verified":true,"digest":digest,"reference":format!("{}/{}@{digest}",config.registry.trim_end_matches('/'),config.repository)}),
    )
}

fn save_state(path: &Path, state: &Value) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(state)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// A Go proxy serves immutable module files. It has no registry upload endpoint.
fn go_publish(config: &GoConfig, record: &Value, files: &Files, completed: bool) -> Result<Value> {
    let root = Path::new(&config.directory);
    fs::create_dir_all(root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".wisp-publish.lock"))?;
    lock.try_lock()
        .context("another publisher holds this Go proxy lock")?;
    let version = format!("v{}", field(record, "version")?);
    let directory = root.join(&config.module).join("@v");
    fs::create_dir_all(&directory)?;
    for (extension, filename) in [("zip", "go.zip"), ("mod", "go.mod"), ("info", "go.info")] {
        let path = directory.join(format!("{version}.{extension}"));
        let bytes = &files[filename];
        if path.try_exists()? {
            ensure!(
                read(&path, LIMIT)? == *bytes,
                "Go proxy version content conflict: {}",
                path.display()
            );
        } else {
            ensure!(!completed, "previously published Go module file missing");
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            file.write_all(bytes)?;
            file.as_file().sync_all()?;
            file.persist_noclobber(&path)?;
            ensure!(read(&path, LIMIT)? == *bytes, "Go proxy readback differs");
        }
    }
    let list = directory.join("list");
    let previous = if list.try_exists()? {
        String::from_utf8(read(&list, 1024 * 1024)?)?
    } else {
        String::new()
    };
    let mut versions = previous
        .lines()
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    for version in &versions {
        let parsed = semver::Version::parse(
            version
                .strip_prefix('v')
                .context("invalid Go proxy version list")?,
        )?;
        ensure!(
            format!("v{parsed}") == *version,
            "noncanonical Go proxy version"
        );
    }
    if versions.insert(version.clone()) {
        ensure!(
            !completed,
            "previously published Go version missing from list"
        );
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(
            format!("{}\n", versions.into_iter().collect::<Vec<_>>().join("\n")).as_bytes(),
        )?;
        file.as_file().sync_all()?;
        file.persist(&list)?;
    }
    fs::File::open(&directory)?.sync_all()?;
    Ok(
        json!({"verified":true,"directory":root.canonicalize()?,"module":config.module,"version":version}),
    )
}

pub fn publish(plan_path: &Path, state_path: &Path) -> Result<Value> {
    let reviewed_bytes = read(plan_path, 4 * 1024 * 1024)?;
    let reviewed = crate::release::strict_json(&reviewed_bytes)?;
    let config = config(&reviewed["registries"])?;
    let (record, files) = snapshot(Path::new(field(&reviewed, "releaseDirectory")?))?;
    ensure!(
        reviewed
            == plan(
                Path::new(field(&reviewed, "releaseDirectory")?),
                &config,
                &record,
                &files
            )?,
        "publication plan differs from validated release; prepare a new plan"
    );
    let plan_hash = hash(&reviewed_bytes);
    let lock_path = state_path.with_extension("lock");
    ensure!(
        lock_path != plan_path && lock_path != state_path && state_path != plan_path,
        "publication plan/state paths overlap"
    );
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock()
        .context("another publisher holds this state lock")?;
    let mut state = if state_path.try_exists()? {
        crate::release::strict_json(&read(state_path, 1024 * 1024)?)?
    } else {
        json!({"schema":1,"planSha256":plan_hash,"complete":false,"registries":{},"lastError":null})
    };
    crate::release::exact(
        &state,
        &[
            "schema",
            "planSha256",
            "complete",
            "registries",
            "lastError",
        ],
        "publication state",
    )?;
    ensure!(
        state["schema"] == 1 && state["planSha256"] == plan_hash && state["registries"].is_object(),
        "publication state belongs to a different plan"
    );
    let http = Http::new()?;
    let mut registries = vec!["component", "cargo", "npm"];
    if config.go.is_some() {
        registries.push("go");
    }
    for registry in registries {
        let completed = state["registries"][registry]["verified"] == true;
        let result = match registry {
            "component" => component_publish(
                &http,
                &config.component,
                &reviewed["component"]["manifest"],
                &files,
                completed,
            ),
            "cargo" => cargo_publish(&http, &config.cargo, &record, &files, completed),
            "npm" => npm_publish(&http, &config.npm, &record, &files, completed),
            "go" => go_publish(config.go.as_ref().unwrap(), &record, &files, completed),
            _ => unreachable!(),
        };
        match result {
            Ok(result) => {
                state["registries"][registry] = result;
                state["lastError"] = Value::Null;
                save_state(state_path, &state)?;
            }
            Err(error) => {
                state["complete"] = false.into();
                state["lastError"] = json!({"registry":registry,"message":format!("{error:#}")});
                save_state(state_path, &state)?;
                bail!(
                    "{registry} publication failed; completed registry writes are recorded in {}: {error:#}",
                    state_path.display()
                );
            }
        }
    }
    state["complete"] = true.into();
    save_state(state_path, &state)?;
    Ok(state)
}

pub fn run_cli(args: &[String]) -> Result<Value> {
    if args.len() == 6 && args[0] == "fetch" && args[2] == "--digest" && args[4] == "--out" {
        return fetch(Path::new(&args[1]), &args[3], Path::new(&args[5]));
    }
    if args.len() == 6 && args[0] == "prepare" && args[2] == "--config" && args[4] == "--out" {
        return prepare(
            Path::new(&args[1]),
            Path::new(&args[3]),
            Path::new(&args[5]),
        );
    }
    if args.len() == 4 && args[0] == "publish" && args[2] == "--state" {
        return publish(Path::new(&args[1]), Path::new(&args[3]));
    }
    bail!(
        "usage: wisp library prepare DIR --config CONFIG --out PLAN | wisp library publish PLAN --state STATE"
    )
}

#[cfg(test)]
mod go_tests {
    use super::*;
    #[test]
    fn go_proxy_publication_preserves_versions_and_checks_completed_retries() {
        let temp = tempfile::tempdir().unwrap();
        let config = GoConfig {
            directory: temp.path().to_str().unwrap().into(),
            module: "wisp.invalid/wisp/catalog".into(),
        };
        let files = Files::from([
            ("go.zip".into(), b"zip".to_vec()),
            (
                "go.mod".into(),
                b"module wisp.invalid/wisp/catalog\n".to_vec(),
            ),
            ("go.info".into(), b"info".to_vec()),
        ]);
        let record = json!({"version":"1.2.3"});
        assert!(go_publish(&config, &record, &files, false).unwrap()["verified"] == true);
        let dir = temp.path().join("wisp.invalid/wisp/catalog/@v");
        assert_eq!(fs::read(dir.join("list")).unwrap(), b"v1.2.3\n");
        go_publish(&config, &record, &files, true).unwrap();
        fs::write(dir.join("v1.2.3.zip"), b"changed").unwrap();
        assert!(
            go_publish(&config, &record, &files, true)
                .unwrap_err()
                .to_string()
                .contains("conflict")
        );
    }

    #[test]
    fn go_proxy_retries_a_partial_file_set_without_overwriting_it() {
        let temp = tempfile::tempdir().unwrap();
        let config = GoConfig {
            directory: temp.path().to_str().unwrap().into(),
            module: "wisp.invalid/wisp/example".into(),
        };
        let files = Files::from([
            ("go.zip".into(), vec![1, 2]),
            ("go.mod".into(), vec![3]),
            ("go.info".into(), vec![4]),
        ]);
        let dir = temp.path().join("wisp.invalid/wisp/example/@v");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("v0.1.0.zip"), &files["go.zip"]).unwrap();
        go_publish(&config, &json!({"version":"0.1.0"}), &files, false).unwrap();
        assert_eq!(fs::read(dir.join("v0.1.0.mod")).unwrap(), vec![3]);
        go_publish(&config, &json!({"version":"0.2.0"}), &files, false).unwrap();
        assert_eq!(fs::read(dir.join("list")).unwrap(), b"v0.1.0\nv0.2.0\n");
    }
}
