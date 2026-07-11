use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::Context;
use ratatui::text::Span;
use serde::{Deserialize, Serialize};

use crate::active_suggestions::{
    ActiveSuggestionsBuilder, ProcessedSuggestion, SuggestionDescription,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FlycompOptionModel {
    pub metadata: FlycompMetadata,
    pub command: FlycompCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FlycompMetadata {
    pub flycomp_version: String,
    pub command_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FlycompCommand {
    pub name: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub args: Vec<FlycompArg>,
    #[serde(default)]
    pub subcommands: Vec<FlycompCommand>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FlycompArg {
    pub long: Option<String>,
    pub short: Option<String>,
    pub description: Option<String>,
    pub value_name: Option<String>,
    pub num_args: Option<String>,
    pub value_enum: Option<Vec<String>>,
    #[serde(default)]
    pub value_hint: String,
}

const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExecutableFingerprint {
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedOptionModel {
    schema_version: u32,
    command_identity: String,
    executable: Option<ExecutableFingerprint>,
    model: FlycompOptionModel,
}

impl FlycompArg {
    fn takes_value(&self) -> bool {
        self.value_name.is_some()
            || self.num_args.is_some()
            || self.short.as_ref().is_some_and(|flag| flag.ends_with('#'))
            || self.long.as_ref().is_some_and(|flag| flag.ends_with('#'))
    }

    fn display_description(&self) -> String {
        let mut details = Vec::new();
        if let Some(value_name) = self.value_name.as_deref() {
            let value_name = value_name.trim();
            if !value_name.is_empty() {
                if value_name.starts_with(['<', '[']) {
                    details.push(value_name.to_string());
                } else {
                    details.push(format!("<{value_name}>"));
                }
            }
        } else if self.takes_value() {
            details.push("<VALUE>".to_string());
        }

        if let Some(values) = self.value_enum.as_ref().filter(|values| !values.is_empty()) {
            details.push(format!("[{}]", values.join("|")));
        }

        if let Some(description) = self.description.as_deref() {
            let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
            if !description.is_empty() {
                details.push(description);
            }
        }
        details.join("  ")
    }
}

pub(crate) fn parse_flycomp_json(json: &str) -> anyhow::Result<FlycompOptionModel> {
    let model: FlycompOptionModel =
        serde_json::from_str(json).context("invalid flycomp JSON output")?;
    validate_model(&model)?;
    Ok(model)
}

fn validate_model(model: &FlycompOptionModel) -> anyhow::Result<()> {
    if model.metadata.flycomp_version.trim().is_empty() {
        anyhow::bail!("flycomp JSON metadata has no version");
    }
    if model.metadata.command_path.trim().is_empty() {
        anyhow::bail!("flycomp JSON metadata has no command path");
    }
    if model
        .command
        .name
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        anyhow::bail!("flycomp JSON has no command name");
    }
    Ok(())
}

fn command_for_context<'a>(
    root: &'a FlycompCommand,
    context_before_word: &str,
) -> &'a FlycompCommand {
    let Some(words) = shlex::split(context_before_word) else {
        return root;
    };

    let mut command = root;
    for word in words.into_iter().skip(1) {
        if let Some(subcommand) = command.subcommands.iter().find(|subcommand| {
            subcommand.name.as_deref() == Some(word.as_str())
                || subcommand.aliases.iter().any(|alias| alias == &word)
        }) {
            command = subcommand;
        }
    }
    command
}

pub(crate) fn suggestions_from_model(
    model: &FlycompOptionModel,
    context_before_word: &str,
    typed_prefix: &str,
) -> ActiveSuggestionsBuilder {
    let command = command_for_context(&model.command, context_before_word);
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();

    for arg in &command.args {
        let description_text = arg.display_description();
        let description = if description_text.is_empty() {
            SuggestionDescription::Static(Vec::new())
        } else {
            SuggestionDescription::Static(vec![Span::raw(description_text)])
        };

        for flag in [arg.long.as_deref(), arg.short.as_deref()]
            .into_iter()
            .flatten()
        {
            let flag = flag.trim_end_matches('#');
            if !flag.starts_with('-')
                || !flag.starts_with(typed_prefix)
                || !seen.insert(flag.to_string())
            {
                continue;
            }
            suggestions.push(
                ProcessedSuggestion::new(flag, "", " ").with_description(description.clone()),
            );
        }
    }

    let mut builder = ActiveSuggestionsBuilder::from_processed(suggestions);
    builder.set_common_prefix();
    builder
}

fn executable_fingerprint(command_identity: &str) -> Option<ExecutableFingerprint> {
    let metadata = std::fs::metadata(command_identity).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(ExecutableFingerprint {
        len: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

pub(crate) fn option_cache_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("flyline").join("flycomp-options"))
}

fn cache_path(root: &Path, command_identity: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    command_identity.hash(&mut hasher);
    let hash = hasher.finish();
    let name = Path::new(command_identity)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("command")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    root.join(format!("{name}-{hash:016x}.json"))
}

pub(crate) fn load_cached_model(command_identity: &str) -> Option<FlycompOptionModel> {
    let root = option_cache_root()?;
    load_cached_model_from(&root, command_identity)
}

fn load_cached_model_from(root: &Path, command_identity: &str) -> Option<FlycompOptionModel> {
    let path = cache_path(root, command_identity);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::debug!("Could not read flycomp option cache {path:?}: {error}");
            return None;
        }
    };
    let cached: CachedOptionModel = match serde_json::from_str(&contents) {
        Ok(cached) => cached,
        Err(error) => {
            log::warn!("Ignoring malformed flycomp option cache {path:?}: {error}");
            return None;
        }
    };
    if cached.schema_version != CACHE_SCHEMA_VERSION
        || cached.command_identity != command_identity
        || cached.executable != executable_fingerprint(command_identity)
        || validate_model(&cached.model).is_err()
    {
        log::debug!("Ignoring stale flycomp option cache {path:?}");
        return None;
    }
    Some(cached.model)
}

pub(crate) fn store_cached_model(
    command_identity: &str,
    model: &FlycompOptionModel,
) -> anyhow::Result<()> {
    let Some(root) = option_cache_root() else {
        anyhow::bail!("neither XDG_CACHE_HOME nor HOME is set");
    };
    store_cached_model_at(&root, command_identity, model)
}

fn store_cached_model_at(
    root: &Path,
    command_identity: &str,
    model: &FlycompOptionModel,
) -> anyhow::Result<()> {
    validate_model(model)?;
    if model.metadata.command_path != command_identity {
        anyhow::bail!(
            "flycomp JSON command path {:?} does not match {:?}",
            model.metadata.command_path,
            command_identity
        );
    }

    std::fs::create_dir_all(root)
        .with_context(|| format!("could not create flycomp option cache {root:?}"))?;
    let path = cache_path(root, command_identity);
    let cached = CachedOptionModel {
        schema_version: CACHE_SCHEMA_VERSION,
        command_identity: command_identity.to_string(),
        executable: executable_fingerprint(command_identity),
        model: model.clone(),
    };
    let json = serde_json::to_vec_pretty(&cached).context("could not serialize option cache")?;
    let temp_path = root.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("flycomp-options"),
        std::process::id()
    ));

    let write_result = (|| -> anyhow::Result<()> {
        let mut file = std::fs::File::create(&temp_path)
            .with_context(|| format!("could not create temporary cache {temp_path:?}"))?;
        file.write_all(&json)
            .with_context(|| format!("could not write temporary cache {temp_path:?}"))?;
        file.sync_all()
            .with_context(|| format!("could not flush temporary cache {temp_path:?}"))?;
        std::fs::rename(&temp_path, &path)
            .with_context(|| format!("could not replace option cache {path:?}"))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json(command_path: &str) -> String {
        format!(
            r#"{{
                "metadata": {{
                    "flycomp_version": "1.1.0",
                    "command_path": {command_path:?},
                    "generated_at": "2026-07-11T00:00:00Z"
                }},
                "command": {{
                    "name": "tool",
                    "args": [
                        {{
                            "long": "--color",
                            "short": "-c",
                            "description": "choose output color",
                            "value_name": "WHEN",
                            "value_enum": ["always", "never"]
                        }},
                        {{
                            "long": "--verbose",
                            "short": "-v",
                            "description": "show more detail"
                        }}
                    ],
                    "subcommands": [
                        {{
                            "name": "remote",
                            "aliases": ["r"],
                            "args": [
                                {{
                                    "long": "--delete",
                                    "description": "delete a remote"
                                }}
                            ]
                        }}
                    ]
                }}
            }}"#
        )
    }

    fn sample_model(command_path: &str) -> FlycompOptionModel {
        parse_flycomp_json(&sample_json(command_path)).unwrap()
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "flyline-flycomp-options-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn parses_and_filters_root_options_with_descriptions() {
        let model = sample_model("tool");
        let mut builder = suggestions_from_model(&model, "tool ", "--col");
        builder.process_all_blocking();

        assert_eq!(builder.processed.len(), 1);
        assert_eq!(builder.processed[0].s, "--color");
        assert_eq!(builder.processed[0].suffix, " ");
        let SuggestionDescription::Static(spans) = &builder.processed[0].description else {
            panic!("expected static description");
        };
        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "<WHEN>  [always|never]  choose output color"
        );
    }

    #[test]
    fn resolves_subcommands_and_aliases() {
        let model = sample_model("tool");
        for context in ["tool remote ", "tool r "] {
            let mut builder = suggestions_from_model(&model, context, "--");
            builder.process_all_blocking();
            assert_eq!(
                builder
                    .processed
                    .iter()
                    .map(|suggestion| suggestion.s.as_str())
                    .collect::<Vec<_>>(),
                vec!["--delete"]
            );
        }
    }

    #[test]
    fn emits_short_and_long_flags_without_duplicates() {
        let mut model = sample_model("tool");
        model.command.args.push(FlycompArg {
            long: Some("--verbose".to_string()),
            description: Some("duplicate".to_string()),
            ..FlycompArg::default()
        });
        let mut builder = suggestions_from_model(&model, "tool ", "-");
        builder.process_all_blocking();
        let values = builder
            .processed
            .iter()
            .map(|suggestion| suggestion.s.as_str())
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["--color", "-c", "--verbose", "-v"]);
    }

    #[test]
    fn rejects_malformed_or_incomplete_json() {
        assert!(parse_flycomp_json("not json").is_err());
        assert!(
            parse_flycomp_json(
                r#"{"metadata":{"flycomp_version":"1","command_path":"tool"},"command":{}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn cache_round_trip_and_atomic_replacement() {
        let root = temp_dir("round-trip");
        let identity = "tool";
        let mut model = sample_model(identity);
        store_cached_model_at(&root, identity, &model).unwrap();
        assert_eq!(
            load_cached_model_from(&root, identity)
                .unwrap()
                .command
                .args
                .len(),
            2
        );

        model.command.args.clear();
        store_cached_model_at(&root, identity, &model).unwrap();
        assert!(
            load_cached_model_from(&root, identity)
                .unwrap()
                .command
                .args
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cache_invalidates_when_executable_changes() {
        let root = temp_dir("fingerprint");
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("tool");
        std::fs::write(&executable, "one").unwrap();
        let identity = executable.to_string_lossy().into_owned();
        let model = sample_model(&identity);
        store_cached_model_at(&root, &identity, &model).unwrap();
        assert!(load_cached_model_from(&root, &identity).is_some());

        std::fs::write(&executable, "different length").unwrap();
        assert!(load_cached_model_from(&root, &identity).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_cache_is_a_miss() {
        let root = temp_dir("malformed");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(cache_path(&root, "tool"), "{broken").unwrap();
        assert!(load_cached_model_from(&root, "tool").is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
