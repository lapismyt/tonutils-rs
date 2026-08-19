//! Deterministic checker for checked-in TON TL and TL-B schemas.
//!
//! The binary compares generated summaries and the schema inventory with the
//! source files committed in the workspace. It does not fetch upstream data or
//! modify a live network. Run `cargo run -p tonutils-schema-gen -- check` from
//! the repository root when validating schema changes.

use std::{
    collections::HashSet,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use thiserror::Error;

const SCHEMA_PATHS: &[(&str, SchemaKind)] = &[
    (
        "crates/tonutils-tl/src/tl/schemas/lite_api.tl",
        SchemaKind::Tl,
    ),
    (
        "crates/tonutils-tl/src/tl/schemas/ton_api.tl",
        SchemaKind::Tl,
    ),
    (
        "crates/tonutils-tl/src/tl/schemas/tonlib_api.tl",
        SchemaKind::Tl,
    ),
    (
        "crates/tonutils-tlb/src/tlb/schemas/block.tlb",
        SchemaKind::Tlb,
    ),
];

const UPSTREAM_COMMIT: &str = "3d478cbde854be03a18ab2a59f8fc3c565cf7d14";

#[derive(Debug, Error)]
enum Error {
    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("schema validation failed for {path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("generated output is stale: {path}")]
    Stale { path: PathBuf },
    #[error("inventory is stale: {0}")]
    StaleInventory(PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SchemaKind {
    Tl,
    Tlb,
}

impl SchemaKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tl => "TL",
            Self::Tlb => "TL-B",
        }
    }

    fn generated_dir(self) -> &'static str {
        match self {
            Self::Tl => "crates/tonutils-tl/src/tl/generated",
            Self::Tlb => "crates/tonutils-tlb/src/tlb/generated",
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Generate and validate checked-in TON schema metadata")]
struct Args {
    /// Rewrite generated metadata and the schema inventory.
    #[arg(long, conflicts_with_all = ["check", "inventory"])]
    write: bool,
    /// Compare regenerated metadata and inventory byte-for-byte.
    #[arg(long, conflicts_with_all = ["write", "inventory"])]
    check: bool,
    /// Validate the schema inventory and print its deterministic contents.
    #[arg(long, conflicts_with_all = ["write", "check"])]
    inventory: bool,
    /// Repository root. Defaults to the workspace containing this package.
    #[arg(long, value_name = "PATH")]
    root: Option<PathBuf>,
}

#[derive(Debug)]
struct Schema {
    relative_path: &'static str,
    kind: SchemaKind,
    hash: String,
    constructors: Vec<Constructor>,
}

impl Schema {
    fn revision(&self) -> String {
        format!(
            "upstream:{};sha256:{};constructors:{}",
            UPSTREAM_COMMIT,
            self.hash,
            self.constructors.len()
        )
    }
}

#[derive(Debug)]
struct Constructor {
    name: String,
    tag: String,
    result: String,
    field_count: usize,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();
    let root = args.root.unwrap_or_else(workspace_root);
    let schemas = load_schemas(&root)?;
    let inventory = render_inventory(&schemas);

    if args.inventory {
        print!("{inventory}");
        return Ok(());
    }

    if args.write {
        for schema in &schemas {
            let path = generated_path(&root, schema);
            let contents = render_module(schema);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            fs::write(&path, contents).map_err(|source| Error::Io { path, source })?;
        }
        let path = root.join("dev-docs/schema-inventory.tsv");
        fs::write(&path, inventory).map_err(|source| Error::Io { path, source })?;
        return Ok(());
    }

    check_outputs(&root, &schemas, &inventory)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("schema generator is nested under the workspace root")
        .to_path_buf()
}

fn load_schemas(root: &Path) -> Result<Vec<Schema>, Error> {
    SCHEMA_PATHS
        .iter()
        .map(|(relative_path, kind)| {
            let path = root.join(relative_path);
            let source = fs::read_to_string(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            let constructors = parse_schema(&source, *kind).map_err(|message| Error::Invalid {
                path: path.clone(),
                message,
            })?;
            let hash = hex_digest(source.as_bytes());
            validate_constructors(&constructors).map_err(|message| Error::Invalid {
                path: path.clone(),
                message,
            })?;
            Ok(Schema {
                relative_path,
                kind: *kind,
                hash,
                constructors,
            })
        })
        .collect()
}

fn parse_schema(source: &str, kind: SchemaKind) -> Result<Vec<Constructor>, String> {
    let cleaned = strip_block_comments(source)
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .filter(|line| {
            let line = line.trim();
            !line.starts_with("---") && line != "$"
        })
        .collect::<Vec<_>>()
        .join("\n");
    let statements = cleaned
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect::<Vec<_>>();

    statements
        .into_iter()
        .map(|statement| {
            if !statement.contains('=') {
                return Err(format!(
                    "unsupported schema statement without result type: {statement:?}"
                ));
            }
            parse_constructor(statement, kind)
        })
        .collect()
}

fn strip_block_comments(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_comment = false;
    while let Some(character) = chars.next() {
        if in_comment {
            if character == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_comment = false;
                output.push(' ');
            } else if character == '\n' {
                output.push('\n');
            }
        } else if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_comment = true;
            output.push(' ');
        } else {
            output.push(character);
        }
    }
    output
}

fn parse_constructor(statement: &str, kind: SchemaKind) -> Result<Constructor, String> {
    let (left, result) = statement
        .rsplit_once('=')
        .ok_or_else(|| "missing result separator".to_owned())?;
    let result = result.trim().to_owned();
    let left = left.trim();
    let token = left
        .split_whitespace()
        .next()
        .ok_or_else(|| "missing constructor name".to_owned())?;
    let (name, tag) = match kind {
        SchemaKind::Tl => {
            let (name, tag) = token.split_once('#').unwrap_or((token, ""));
            (name.to_owned(), tag.to_owned())
        }
        SchemaKind::Tlb => {
            if token == "_" {
                ("_".to_owned(), "_".to_owned())
            } else if let Some((name, tag)) = token.split_once('$') {
                (name.to_owned(), format!("${tag}"))
            } else if let Some((name, tag)) = token.split_once('#') {
                (name.to_owned(), format!("#{tag}"))
            } else {
                (token.to_owned(), String::new())
            }
        }
    };
    Ok(Constructor {
        name,
        tag,
        result,
        field_count: left.split_whitespace().count().saturating_sub(1),
    })
}

fn validate_constructors(constructors: &[Constructor]) -> Result<(), String> {
    let mut identities = HashSet::new();
    for constructor in constructors {
        let identity = (&constructor.name, &constructor.tag, &constructor.result);
        if constructor.name != "_" && !identities.insert(identity) {
            return Err(format!(
                "duplicate constructor identity {} {} = {}",
                constructor.name, constructor.tag, constructor.result
            ));
        }
    }
    Ok(())
}

fn render_module(schema: &Schema) -> String {
    let mut out = String::new();
    writeln!(out, "// @generated by tonutils-schema-gen; do not edit.").unwrap();
    writeln!(out, "// source: {}", schema.relative_path).unwrap();
    writeln!(out, "// sha256: {}", schema.hash).unwrap();
    writeln!(out, "// upstream commit: {UPSTREAM_COMMIT}\n").unwrap();
    writeln!(
        out,
        "pub const SCHEMA_PATH: &str = {:?};",
        schema.relative_path
    )
    .unwrap();
    writeln!(
        out,
        "pub const SCHEMA_KIND: &str = {:?};",
        schema.kind.as_str()
    )
    .unwrap();
    writeln!(out, "pub const SOURCE_SHA256: &str = {:?};", schema.hash).unwrap();
    writeln!(
        out,
        "pub const UPSTREAM_COMMIT: &str = {:?};",
        UPSTREAM_COMMIT
    )
    .unwrap();
    writeln!(
        out,
        "pub const CONSTRUCTOR_COUNT: usize = {};",
        schema.constructors.len()
    )
    .unwrap();
    writeln!(
        out,
        "pub const SCHEMA_REVISION: &str = {:?};\n",
        schema.revision()
    )
    .unwrap();
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    out.push_str("pub struct ConstructorMetadata {\n");
    out.push_str("    pub name: &'static str,\n    pub tag: &'static str,\n");
    out.push_str("    pub result: &'static str,\n    pub field_count: usize,\n}\n\n");
    out.push_str("#[rustfmt::skip]\npub const CONSTRUCTORS: &[ConstructorMetadata] = &[\n");
    for constructor in &schema.constructors {
        writeln!(
            out,
            "    ConstructorMetadata {{ name: {:?}, tag: {:?}, result: {:?}, field_count: {} }},",
            constructor.name, constructor.tag, constructor.result, constructor.field_count
        )
        .unwrap();
    }
    out.push_str("];\n");
    out
}

fn render_inventory(schemas: &[Schema]) -> String {
    let mut out = String::from(
        "# Deterministic inventory of checked-in TON schemas\n# path\\tkind\\tsha256\\tconstructors\\trevision\\tgenerated_sha256\\tmodule\n",
    );
    for schema in schemas {
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            schema.relative_path,
            schema.kind.as_str(),
            schema.hash,
            schema.constructors.len(),
            schema.revision(),
            hex_digest(render_module(schema).as_bytes()),
            generated_path_string(schema),
        )
        .unwrap();
    }
    out
}

fn check_outputs(root: &Path, schemas: &[Schema], inventory: &str) -> Result<(), Error> {
    for schema in schemas {
        let path = generated_path(root, schema);
        let expected = render_module(schema);
        let actual = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if actual != expected {
            return Err(Error::Stale { path });
        }
    }
    let path = root.join("dev-docs/schema-inventory.tsv");
    let actual = fs::read_to_string(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    if actual != inventory {
        return Err(Error::StaleInventory(path));
    }
    Ok(())
}

fn generated_path(root: &Path, schema: &Schema) -> PathBuf {
    root.join(generated_path_string(schema))
}

fn generated_path_string(schema: &Schema) -> String {
    let stem = Path::new(schema.relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("schema path has a UTF-8 file stem");
    format!("{}/schema_{stem}.rs", schema.kind.generated_dir())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut output, byte| {
            use std::fmt::Write;

            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tl_namespace_and_explicit_id() {
        let constructors = parse_schema(
            "liteServer.answer#1234 query_id:int = liteServer.Answer;\n",
            SchemaKind::Tl,
        )
        .unwrap();
        assert_eq!(constructors[0].name, "liteServer.answer");
        assert_eq!(constructors[0].tag, "1234");
        assert_eq!(constructors[0].field_count, 1);
    }

    #[test]
    fn parses_tlb_binary_hex_and_implicit_tags() {
        let constructors = parse_schema(
            "foo$00 value:uint32 = Foo; bar#deadbeef = Bar; _ value:uint8 = Baz;",
            SchemaKind::Tlb,
        )
        .unwrap();
        assert_eq!(constructors[0].tag, "$00");
        assert_eq!(constructors[1].tag, "#deadbeef");
        assert_eq!(constructors[2].tag, "_");
    }

    #[test]
    fn rejects_duplicate_constructor_identity() {
        let constructors = vec![
            Constructor {
                name: "foo".into(),
                tag: "#1".into(),
                result: "Foo".into(),
                field_count: 0,
            },
            Constructor {
                name: "foo".into(),
                tag: "#1".into(),
                result: "Foo".into(),
                field_count: 1,
            },
        ];
        assert!(validate_constructors(&constructors).is_err());
    }

    #[test]
    fn rejects_unsupported_statement_instead_of_dropping_it() {
        let error = parse_schema("future_syntax_without_result;", SchemaKind::Tlb)
            .expect_err("unsupported syntax must fail validation");
        assert!(error.contains("without result type"));
    }
}
