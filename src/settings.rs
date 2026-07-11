use std::collections::{HashMap, HashSet};

use crate::app::actions;
use crate::content_builder::TaggedSpan;
use crate::cursor::CursorConfig;
use crate::history::HistoryManager;
use crate::palette::Palette;
use crate::tutorial::TutorialStep;
use clap::ValueEnum;

/// Which theme the user has configured for the colour palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ColourTheme {
    /// Dark-terminal preset (the original flyline palette). This is the default.
    #[default]
    Dark,
    /// Light-terminal preset.
    Light,
}

/// How suggestions should be sorted when fuzzy scores are tied.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, serde::Serialize, serde::Deserialize,
)]
pub enum SuggestionSortOrder {
    /// Sort by last modification time (if available), then alphabetically.
    #[default]
    Mtime,
    /// Sort alphabetically.
    Alphabetical,
}

/// Controls fuzzy matching behavior for suggestions.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, serde::Serialize, serde::Deserialize,
)]
pub enum FuzzyMode {
    /// Enable fuzzy matching for all completions.
    #[default]
    #[value(name = "all")]
    #[serde(rename = "all")]
    All,
    /// Disable fuzzy matching (use prefix matching instead).
    #[value(name = "none")]
    #[serde(rename = "none")]
    None,
    /// Match folders using prefix matching instead of fuzzy matching.
    #[value(name = "folder-prefixes")]
    #[serde(rename = "folder-prefixes")]
    FolderPrefixes,
}

/// A single custom prompt animation registered with `flyline create-prompt-widget animation`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptAnimation {
    /// Name used as placeholder in prompt strings (e.g., `COOL_SPINNER`).
    pub name: String,
    /// Playback speed in frames per second.
    pub fps: f64,
    /// Animation frames.  May contain actual ANSI escape sequences (ESC byte, i.e. `\x1b`).
    pub frames: Vec<String>,
    /// When true the animation reverses direction at each end instead of
    /// wrapping around (ping-pong / bounce mode).
    pub ping_pong: bool,
}

/// A custom prompt widget registered with `flyline create-prompt-widget`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PromptWidget {
    /// Show different text depending on whether mouse capture is enabled.
    MouseMode {
        /// Name used as placeholder in prompt strings (e.g., `FLYLINE_MOUSE_MODE`).
        name: String,
        /// Text shown when mouse capture is enabled.
        enabled_text: String,
        /// Text shown when mouse capture is disabled.
        disabled_text: String,
    },
    /// Copies the current command buffer to the clipboard when clicked.
    CopyBuffer {
        /// Name used as placeholder in prompt strings (e.g., `FLYLINE_COPY_BUFFER`).
        name: String,
        /// Text shown in the prompt.
        text: String,
    },
    /// Runs a shell command and displays its output. Kept as a named struct
    /// because methods/helpers (e.g. `resolve_placeholder`) take `&PromptWidgetCustom`
    /// directly.
    Custom(PromptWidgetCustom),
    /// Shows how long ago the flyline app last closed.
    ///
    /// The elapsed duration is formatted as a compact human-readable string,
    /// for example `9.2s`, `1m23s`, `1h02m03s`, `1d20h43m`.
    LastCommandDuration {
        /// Name used as placeholder in prompt strings (e.g., `FLYLINE_LAST_COMMAND_DURATION`).
        name: String,
    },
}

impl PromptWidget {
    /// The placeholder name that is replaced inside prompt strings (PS1, RPS1, PS1_FILL).
    pub fn name(&self) -> &str {
        match self {
            PromptWidget::MouseMode { name, .. } => name,
            PromptWidget::CopyBuffer { name, .. } => name,
            PromptWidget::Custom(w) => &w.name,
            PromptWidget::LastCommandDuration { name } => name,
        }
    }
}

/// What to show as a placeholder while a non-blocking (or timed-out blocking)
/// custom widget command is still running.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum Placeholder {
    /// Show N spaces.
    Spaces(usize),
    /// Show the previous output of the command (empty on the very first run).
    #[default]
    Prev,
}

/// A prompt widget that runs a shell command and displays its output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptWidgetCustom {
    /// Name used as placeholder in prompt strings (e.g., `CUSTOM_WIDGET1`).
    pub name: String,
    /// Command (and arguments) to run.
    pub command: Vec<String>,
    /// Timeout in milliseconds to wait for the command before rendering the
    /// first prompt frame.  `None` (not specified) defaults to `0`, meaning a
    /// single non-blocking `try_wait` is performed at spawn time — the command
    /// immediately goes to the background if it hasn't finished.  `Some(n)`
    /// polls for up to `n` milliseconds; `Some(i32::MAX)` (~24.8 days) is
    /// effectively indefinite.
    pub block: Option<i32>,
    /// What to show while the command is running (or has timed out).
    pub placeholder: Placeholder,
    /// Most recent successful output of the command; shared across clones so
    /// that the `Placeholder::Prev` option can pick it up on subsequent renders.
    ///
    /// Runtime-only cache: reset to empty on load.
    #[serde(skip)]
    pub prev_output: std::sync::Arc<std::sync::Mutex<Vec<TaggedSpan<'static>>>>,
}

/// A configured agent-mode command with its optional system prompt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentModeCommand {
    /// Command (and arguments) to invoke. The current buffer is appended as the
    /// final argument.  Stored as a `Vec<String>` after splitting the
    /// user-supplied command string on whitespace.
    pub command: Vec<String>,
    /// Optional system prompt prepended to the buffer when invoking AI mode.
    /// When set, the subprocess receives `"<system_prompt>\n<buffer>"` as its final argument.
    pub system_prompt: Option<String>,
}

/// (De)serialization for [`Settings::agent_commands`].  A JSON object cannot key
/// on `Option<String>` (the `None` default command has no string key), so the
/// map is represented on disk as an array of `[key, command]` pairs, where
/// `key` is either `null` or a prefix string.
mod agent_commands_serde {
    use super::{AgentModeCommand, HashMap};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        map: &HashMap<Option<String>, AgentModeCommand>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let pairs: Vec<(&Option<String>, &AgentModeCommand)> = map.iter().collect();
        pairs.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HashMap<Option<String>, AgentModeCommand>, D::Error> {
        let pairs: Vec<(Option<String>, AgentModeCommand)> = Vec::deserialize(deserializer)?;
        Ok(pairs.into_iter().collect())
    }
}

/// Controls whether and when the matrix animation is shown.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MatrixAnimation {
    /// Never show the matrix animation.
    #[default]
    Off,
    /// Always show the matrix animation.
    On,
    /// Show the matrix animation only after the given number of seconds of inactivity
    /// (no keypress or mouse event).
    IdleSecs(u64),
}

/// Controls how flyline manages mouse capture.
#[derive(
    clap::ValueEnum, Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
pub enum MouseMode {
    /// Never capture mouse events.
    Disabled,
    /// Mouse capture is on by default; toggled when Escape is pressed.
    Simple,
    /// Mouse capture is on by default with automatic management: disabled on scroll or when the
    /// user clicks above the viewport, re-enabled on any keypress or when focus is regained.
    /// Also can manually toggle with Escape.
    #[default]
    Smart,
}

/// How many shell integration escape codes (OSC 133 / OSC 633) flyline sends.
#[derive(
    clap::ValueEnum,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ShellIntegrationLevel {
    /// Send no shell integration codes.
    None,
    /// Only send the escape codes that report prompt start/end positions.
    #[default]
    OnlyPromptPos,
    /// Send the full set of shell integration codes: prompt positions, execution
    /// start/end codes, and cursor-position reporting.
    Full,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Optional path to the Zsh history file. When `None`, Zsh history is not loaded.
    /// When `Some`, Zsh history is loaded in addition to Bash history; an empty string or no
    /// value means use the default path (`$HOME/.zsh_history`).
    ///
    /// Runtime-only: re-derived per host from the environment in `Default`, so it
    /// is not persisted.
    #[serde(skip)]
    pub zsh_history_path: Option<String>,
    /// Whether the interactive tutorial is active.
    ///
    /// Transient, per-terminal-session state — NOT durable config. It is
    /// deliberately excluded from the persisted settings snapshot (which is
    /// global and permanent) so that starting the tutorial in one terminal does
    /// not force it onto every other terminal, now and forever. Per-prompt
    /// hosts carry it across their processes via [`SessionState`] instead; the
    /// long-lived Bash builtin keeps it live in-process. See [`SessionState`].
    #[serde(skip)]
    pub run_tutorial: bool,
    /// Current tutorial step. Transient session state; see [`run_tutorial`].
    ///
    /// [`run_tutorial`]: Settings::run_tutorial
    #[serde(skip)]
    pub tutorial_step: TutorialStep,
    /// Whether to show all animations (cursor movement, cursor fading, dynamic time).
    pub show_animations: bool,
    /// Whether to show inline history suggestions.
    pub show_inline_history: bool,
    /// Whether to auto-start tab completion suggestions as you type.
    pub auto_suggest: bool,
    /// Whether to use flycomp to synthesize completions.
    pub use_flycomp: bool,
    /// Whether to offer flycomp option synthesis when a native completer *is*
    /// registered but returns nothing for an option-shaped word (`-`/`--`).
    ///
    /// Many tools ship a completer that lags their `--help` (e.g. BSD `grep`,
    /// whose zsh/bash completers target GNU options), so `grep --<Tab>` yields
    /// nothing. When `true`, flyline treats that empty result as a cue to offer
    /// synthesizing options from `--help`, instead of silently falling back to
    /// filename completion. It does NOT change behaviour for non-option words
    /// (e.g. `kubectl get <Tab>` stays silent, since an empty result there is
    /// contextual, not a missing-options signal). Requires `use_flycomp`.
    pub flycomp_synthesize_options: bool,
    /// Optional path to the directory where flycomp output is saved.
    /// When `None`, defaults to `~/.local/share/bash-completion/completions/`.
    pub flycomp_output: Option<String>,
    /// How to sort suggestions when fuzzy scores are tied.
    pub suggestion_sort_order: SuggestionSortOrder,
    /// Controls fuzzy matching behavior for suggestions.
    pub fuzzy_mode: FuzzyMode,
    /// Maximum number of suggestion rows to render for tab-completion lists.
    pub num_suggestion_rows: u16,
    /// Whether to automatically close opening characters (e.g., parentheses, brackets, quotes).
    pub auto_close_chars: bool,
    /// Whether mouse clicks and drags on the command buffer change the cursor
    /// position and selection. When `false`, mouse interaction with the buffer
    /// does not change the buffer selection or cursor position.
    pub select_with_mouse: bool,
    /// Cursor appearance and animation settings (set via `flyline set-cursor`).
    pub cursor_config: CursorConfig,
    /// Mouse capture mode.
    pub mouse_mode: MouseMode,
    /// Agent-mode commands keyed by optional trigger prefix.
    /// - `None` key: the default command invoked via Alt+Enter (no prefix match needed).
    /// - `Some(prefix)` key: activated when the user presses Enter and the buffer starts
    ///   with `prefix`; the prefix is stripped before the buffer is sent to the command.
    ///
    /// Persisted as an array of `[key, command]` pairs (the `Option<String>` key
    /// cannot be a JSON object key); see [`agent_commands_serde`].
    #[serde(with = "agent_commands_serde")]
    pub agent_commands: HashMap<Option<String>, AgentModeCommand>,
    /// Custom prompt animations registered with `flyline create-prompt-widget animation`.
    pub custom_animations: HashMap<String, PromptAnimation>,
    /// Custom prompt widgets registered with `flyline create-prompt-widget`.
    pub custom_prompt_widgets: HashMap<String, PromptWidget>,
    /// Run matrix animation in the terminal background.
    pub matrix_animation: MatrixAnimation,
    /// Render frame rate in frames per second (1–120).
    pub frame_rate: u8,
    /// Shell integration escape codes level (OSC 133 / OSC 633).
    pub send_shell_integration_codes: ShellIntegrationLevel,
    /// Whether to request the use of extended (kitty-protocol) keyboard codes
    /// during startup. Enabling this gives flyline more accurate keyboard
    /// events on terminals that support the protocol; disable it if your
    /// terminal misbehaves when the request is sent. Enabled by default.
    pub enable_extended_key_codes: bool,
    /// Blacklist of command words for which flycomp prompt should be bypassed.
    pub flycomp_blacklist: HashSet<String>,
    /// Configurable colour palette for UI elements.
    pub colour_palette: Palette,
    /// User defined keybindings.
    ///
    /// Persisted via each binding's human-readable string form (keys like
    /// `ctrl+s`, the context expression, and the camelCase action name).
    pub keybindings: Vec<actions::Binding>,
    /// User defined key remappings (applied before matching bindings).
    ///
    /// Persisted as `{from, to}` string pairs.
    pub key_remappings: Vec<actions::KeyRemap>,
    /// Show the last key event and dispatched action above the prompt.
    pub key_debug: bool,
    /// Show the last mouse event above the prompt.
    pub mouse_debug: bool,
    /// Whether to change the mouse cursor shape depending on what is hovered.
    pub mouse_change_shape: bool,
    /// Tracks commands that were cancelled via Ctrl+C (non-empty buffer).
    #[serde(skip)]
    pub cancelled_command_history_manager: HistoryManager,
    /// Tracks prompts that were submitted to agent mode.
    #[serde(skip)]
    pub agent_prompt_history_manager: HistoryManager,
    /// Timestamp of the most recent flyline app session close.
    ///
    /// Set to `Some(Instant::now())` immediately after each `app::get_command`
    /// call returns. Used by the `last-command-duration` prompt widget to
    /// compute and display the elapsed time since the last command.
    #[serde(skip)]
    pub last_app_closed_at: Option<std::time::Instant>,
    /// Initial buffer content to pre-fill the command line when Flyline starts.
    #[serde(skip)]
    pub initial_buffer: Option<String>,
}

/// Transient, per-terminal-session state for hosts that run flyline as a fresh
/// process on every prompt (e.g. the standalone editor).
///
/// This is deliberately separate from [`Settings`]: [`Settings`] is durable,
/// global config that lives forever in one shared file, whereas this captures
/// state that must survive across a single terminal's per-prompt processes but
/// must NOT leak into other terminals or outlive the terminal — most notably
/// the interactive tutorial. Per-prompt hosts persist it to a session-scoped
/// location (keyed to the terminal session); the long-lived Bash builtin keeps
/// the equivalent state live in-process and does not use this at all.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SessionState {
    /// Whether the interactive tutorial is active in this terminal session.
    pub run_tutorial: bool,
    /// The tutorial step reached in this terminal session.
    pub tutorial_step: TutorialStep,
}

impl SessionState {
    /// True when there is nothing worth persisting for the session (the
    /// tutorial is not running), so the backing file can be removed.
    pub fn is_empty(&self) -> bool {
        !self.run_tutorial && !self.tutorial_step.is_active()
    }
}

impl Settings {
    /// True when flyline runs as the standalone zsh line editor (`FLYLINE_HOST=zsh`).
    pub fn is_zsh_host() -> bool {
        crate::shell::is_zsh_host_env()
    }

    /// Advance the interactive tutorial when the user submits an empty command
    /// while a tutorial step is active, disabling the tutorial once it ends.
    ///
    /// Host-agnostic: every host calls this after an accepted command so the
    /// tutorial progresses identically whether flyline runs as a long-lived
    /// Bash builtin or as a per-prompt standalone process.
    pub fn advance_tutorial_on_submit(&mut self, command: &str) {
        if self.tutorial_step.is_active() && command.trim().is_empty() {
            self.tutorial_step.next();
            log::info!("Tutorial step advanced to {:?}", self.tutorial_step);
            if !self.tutorial_step.is_active() {
                self.run_tutorial = false;
            }
        }
    }

    /// End the interactive tutorial immediately (e.g. when the user cancels
    /// with Ctrl+C) so it does not resume on the next prompt.
    pub fn stop_tutorial(&mut self) {
        self.run_tutorial = false;
        self.tutorial_step = TutorialStep::NotRunning;
    }

    /// Overlay transient session state (e.g. tutorial progress) onto these
    /// settings after the durable snapshot has been loaded.
    pub fn apply_session_state(&mut self, state: &SessionState) {
        self.run_tutorial = state.run_tutorial;
        self.tutorial_step = state.tutorial_step;
    }

    /// Extract the transient, per-terminal-session state from these settings so
    /// a per-prompt host can hand it to the next process in the same terminal.
    pub fn session_state(&self) -> SessionState {
        SessionState {
            run_tutorial: self.run_tutorial,
            tutorial_step: self.tutorial_step,
        }
    }

    fn default_zsh_history_path() -> Option<String> {
        if Self::is_zsh_host() {
            Some(std::env::var("HISTFILE").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                format!("{home}/.zsh_history")
            }))
        } else {
            None
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            zsh_history_path: Self::default_zsh_history_path(),
            run_tutorial: false,
            tutorial_step: TutorialStep::default(),
            show_animations: true,
            auto_suggest: true,
            use_flycomp: true,
            flycomp_synthesize_options: true,
            flycomp_output: None,
            suggestion_sort_order: SuggestionSortOrder::default(),
            fuzzy_mode: FuzzyMode::default(),
            num_suggestion_rows: 15,
            show_inline_history: true,
            auto_close_chars: true,
            select_with_mouse: true,
            cursor_config: CursorConfig::default(),
            mouse_mode: MouseMode::default(),
            agent_commands: HashMap::default(),
            custom_animations: HashMap::default(),
            custom_prompt_widgets: HashMap::default(),
            matrix_animation: MatrixAnimation::default(),
            frame_rate: 24,
            send_shell_integration_codes: ShellIntegrationLevel::default(),
            enable_extended_key_codes: true,
            flycomp_blacklist: HashSet::default(),
            colour_palette: Palette::default(),
            keybindings: Vec::default(),
            key_remappings: Vec::default(),
            key_debug: false,
            mouse_debug: false,
            mouse_change_shape: true,
            cancelled_command_history_manager: HistoryManager::new_empty(),
            agent_prompt_history_manager: HistoryManager::new_empty(),
            last_app_closed_at: None,
            initial_buffer: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_via_json() {
        let mut s = Settings::default();
        s.num_suggestion_rows = 42;
        s.frame_rate = 60;
        s.use_flycomp = false;
        s.mouse_mode = MouseMode::Disabled;

        let json = serde_json::to_string(&s).expect("serialize settings");
        let back: Settings = serde_json::from_str(&json).expect("deserialize settings");

        assert_eq!(back.num_suggestion_rows, 42);
        assert_eq!(back.frame_rate, 60);
        assert!(!back.use_flycomp);
        assert_eq!(back.mouse_mode, MouseMode::Disabled);
    }

    #[test]
    fn settings_partial_json_falls_back_to_defaults() {
        // An old/partial snapshot missing most fields must still load, with the
        // absent fields taking their `Default` values (container `serde(default)`).
        let back: Settings =
            serde_json::from_str(r#"{ "num_suggestion_rows": 7 }"#).expect("deserialize partial");
        assert_eq!(back.num_suggestion_rows, 7);
        assert_eq!(back.frame_rate, Settings::default().frame_rate);
        assert_eq!(back.use_flycomp, Settings::default().use_flycomp);
    }

    #[test]
    fn tutorial_state_is_transient_session_state_not_durable_config() {
        // Tutorial fields are session-scoped, not durable config: they must
        // never appear in the persisted settings snapshot, otherwise starting
        // the tutorial in one terminal would resurface it in every other.
        let mut s = Settings::default();
        s.run_tutorial = true;
        s.tutorial_step = TutorialStep::Welcome;

        let json = serde_json::to_string(&s).expect("serialize settings");
        assert!(!json.contains("run_tutorial"));
        assert!(!json.contains("tutorial_step"));

        let back: Settings = serde_json::from_str(&json).expect("deserialize settings");
        assert!(!back.run_tutorial);
        assert_eq!(back.tutorial_step, TutorialStep::NotRunning);

        // It survives via SessionState instead.
        let round_tripped: SessionState = serde_json::from_str(
            &serde_json::to_string(&s.session_state()).expect("serialize session state"),
        )
        .expect("deserialize session state");
        assert!(round_tripped.run_tutorial);
        assert_eq!(round_tripped.tutorial_step, TutorialStep::Welcome);

        let mut fresh = Settings::default();
        fresh.apply_session_state(&round_tripped);
        assert!(fresh.run_tutorial);
        assert_eq!(fresh.tutorial_step, TutorialStep::Welcome);
    }

    #[test]
    fn tutorial_advances_on_empty_submit_only() {
        let mut s = Settings::default();
        s.run_tutorial = true;
        s.tutorial_step = TutorialStep::Welcome;

        // Empty submit advances a step.
        s.advance_tutorial_on_submit("");
        assert_ne!(s.tutorial_step, TutorialStep::Welcome);
        assert!(s.tutorial_step.is_active());
        assert!(s.run_tutorial);

        // A real command does not advance the tutorial.
        let step = s.tutorial_step;
        s.advance_tutorial_on_submit("ls -la");
        assert_eq!(s.tutorial_step, step);

        // Stepping to the end deactivates the tutorial.
        for _ in 0..20 {
            s.advance_tutorial_on_submit("");
        }
        assert_eq!(s.tutorial_step, TutorialStep::NotRunning);
        assert!(!s.run_tutorial);
    }

    #[test]
    fn stop_tutorial_ends_it_immediately() {
        let mut s = Settings::default();
        s.run_tutorial = true;
        s.tutorial_step = TutorialStep::Welcome;

        s.stop_tutorial();

        assert!(!s.run_tutorial);
        assert_eq!(s.tutorial_step, TutorialStep::NotRunning);
        assert!(s.session_state().is_empty());
    }

    #[test]
    fn settings_skipped_fields_are_not_serialized() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).expect("serialize settings");
        // Genuine runtime-only fields must never appear in the snapshot.
        assert!(!json.contains("initial_buffer"));
        assert!(!json.contains("zsh_history_path"));
        // Formerly-skipped fields are now persisted (present even when empty).
        assert!(json.contains("keybindings"));
        assert!(json.contains("agent_commands"));
        assert!(json.contains("key_remappings"));
    }

    #[test]
    fn settings_keybindings_remappings_agent_commands_round_trip() {
        use crate::app::actions::{Binding, KeyRemap};
        use crossterm::event::{KeyCode, KeyModifiers};

        let mut s = Settings::default();

        // A ctrl+s style binding and an anychar+mods binding.
        s.keybindings = vec![
            Binding::try_new_from_strs("ctrl+s", "always=submitOrNewline").expect("ctrl+s binding"),
            Binding::try_new_from_strs("ctrl+anychar", "tabCompletion=insertChar")
                .expect("anychar binding"),
        ];

        // Both remap variants.
        s.key_remappings = vec![
            KeyRemap::Key {
                from: KeyCode::Tab,
                to: KeyCode::Char('z'),
            },
            KeyRemap::Modifier {
                from: KeyModifiers::ALT,
                to: KeyModifiers::CONTROL,
            },
        ];

        // Agent commands with both a None key and a Some(prefix) key.
        s.agent_commands.insert(
            None,
            AgentModeCommand {
                command: vec!["llm".to_string(), "-m".to_string()],
                system_prompt: Some("be concise".to_string()),
            },
        );
        s.agent_commands.insert(
            Some("? ".to_string()),
            AgentModeCommand {
                command: vec!["explain".to_string()],
                system_prompt: None,
            },
        );

        let json = serde_json::to_string(&s).expect("serialize settings");
        let back: Settings = serde_json::from_str(&json).expect("deserialize settings");

        assert_eq!(back.keybindings, s.keybindings);
        assert_eq!(back.key_remappings, s.key_remappings);
        assert_eq!(back.agent_commands, s.agent_commands);
    }

    #[test]
    fn settings_persisted_fields_are_human_readable_strings() {
        use crate::app::actions::Binding;

        let mut s = Settings::default();
        s.keybindings =
            vec![Binding::try_new_from_strs("ctrl+s", "always=submitOrNewline").unwrap()];

        let json = serde_json::to_string(&s).expect("serialize settings");

        // Human-readable key + action names appear verbatim.
        assert!(json.contains("ctrl+s"));
        assert!(json.contains("submitOrNewline"));
        // Crossterm-internal representations must NOT leak into the snapshot.
        assert!(!json.contains("CONTROL"));
        assert!(!json.contains(r#""code""#));
        assert!(!json.contains(r#""modifiers""#));
    }
}
