//! UI 文字列のローカライズ。
//!
//! UI に出す定型の文字列は [`Msg`] で表し、[`tr`] で言語ごとの対応表から引く。
//! 言語を足すときは [`Lang`] に値を追加し、対応表の関数(`en` と同じ形)を書く。
//! 対応表は `match` で書くので、訳し漏れはコンパイルエラーになる。

/// UI の言語。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
}

/// UI に出す定型の文字列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    ConfigPathNotFound,
    ConfigLoadFailed,
    KeyboardHeading,
    LegendRemapped,
    LegendUnsaved,
    LegendMoreRemaps,
    SelectedKeyHeading,
    NoKeySelected,
    KeyName,
    KeyCode,
    VirtualKey,
    NumpadOrigin,
    Yes,
    No,
    RemappedTo,
    RemappedFrom,
    NoRemapsForKey,
    KeyRemapsHeading,
    RemapTo,
    NotRemapped,
    RemoveRemap,
    Save,
    UnsavedChanges,
    NoUnsavedChanges,
    Saved,
    SavedNotApplied,
    SignalUnsupported,
    SaveAbortedModified,
    SaveAbortedLocked,
    SaveFailedRead,
    SaveFailedBackup,
    SaveFailedWrite,
    BackupDirNotFound,
    UnsafeActiveConfiguration,
    Reload,
    Reloaded,
    ReloadFailed,
    ReloadConfirm,
    DiscardAndReload,
    CloseConfirm,
    DontSave,
    Cancel,
    SignalEventNotFound,
    SettingsLoadFailed,
}

impl Msg {
    /// すべての値(テスト用)。
    #[cfg(test)]
    pub const ALL: &[Msg] = &[
        Msg::ConfigPathNotFound,
        Msg::ConfigLoadFailed,
        Msg::KeyboardHeading,
        Msg::LegendRemapped,
        Msg::LegendUnsaved,
        Msg::LegendMoreRemaps,
        Msg::SelectedKeyHeading,
        Msg::NoKeySelected,
        Msg::KeyName,
        Msg::KeyCode,
        Msg::VirtualKey,
        Msg::NumpadOrigin,
        Msg::Yes,
        Msg::No,
        Msg::RemappedTo,
        Msg::RemappedFrom,
        Msg::NoRemapsForKey,
        Msg::KeyRemapsHeading,
        Msg::RemapTo,
        Msg::NotRemapped,
        Msg::RemoveRemap,
        Msg::Save,
        Msg::UnsavedChanges,
        Msg::NoUnsavedChanges,
        Msg::Saved,
        Msg::SavedNotApplied,
        Msg::SignalUnsupported,
        Msg::SaveAbortedModified,
        Msg::SaveAbortedLocked,
        Msg::SaveFailedRead,
        Msg::SaveFailedBackup,
        Msg::SaveFailedWrite,
        Msg::BackupDirNotFound,
        Msg::UnsafeActiveConfiguration,
        Msg::Reload,
        Msg::Reloaded,
        Msg::ReloadFailed,
        Msg::ReloadConfirm,
        Msg::DiscardAndReload,
        Msg::CloseConfirm,
        Msg::DontSave,
        Msg::Cancel,
        Msg::SignalEventNotFound,
        Msg::SettingsLoadFailed,
    ];
}

/// `msg` を `lang` の文字列にする。
pub fn tr(lang: Lang, msg: Msg) -> &'static str {
    match lang {
        Lang::En => en(msg),
    }
}

fn en(msg: Msg) -> &'static str {
    match msg {
        Msg::ConfigPathNotFound => "Config file path not found",
        Msg::ConfigLoadFailed => "Failed to load the config file",
        Msg::KeyboardHeading => "Keyboard",
        Msg::LegendRemapped => "Remapped",
        Msg::LegendUnsaved => "Changed (not saved yet)",
        Msg::LegendMoreRemaps => "Other codes of the key are also remapped",
        Msg::SelectedKeyHeading => "Selected key",
        Msg::NoKeySelected => "Click a key to see its details.",
        Msg::KeyName => "Name",
        Msg::KeyCode => "Key code",
        Msg::VirtualKey => "Virtual key",
        Msg::NumpadOrigin => "From numpad",
        Msg::Yes => "Yes",
        Msg::No => "No",
        Msg::RemappedTo => "Remapped to",
        Msg::RemappedFrom => "Remapped from",
        Msg::NoRemapsForKey => "No remaps for this key.",
        Msg::KeyRemapsHeading => "Key remaps",
        Msg::RemapTo => "Remap to",
        Msg::NotRemapped => "(not remapped)",
        Msg::RemoveRemap => "Remove",
        Msg::Save => "Save",
        Msg::UnsavedChanges => "Unsaved changes",
        Msg::NoUnsavedChanges => "No unsaved changes",
        Msg::Saved => "Saved.",
        Msg::SavedNotApplied => {
            "Saved, but PowerToys may not have applied the changes (turn Keyboard Manager off and on in PowerToys to apply them)"
        }
        Msg::SignalUnsupported => "notifying PowerToys is only supported on Windows",
        Msg::SaveAbortedModified => {
            "Not saved: the file was changed by another program after it was loaded."
        }
        Msg::SaveAbortedLocked => "Not saved: the file is locked by another program (timed out)",
        Msg::SaveFailedRead => "Not saved: failed to read the current file",
        Msg::SaveFailedBackup => "Not saved: failed to create a backup",
        Msg::SaveFailedWrite => "Failed to save",
        Msg::BackupDirNotFound => {
            "Not saved: the backup folder (next to the executable) could not be determined."
        }
        Msg::UnsafeActiveConfiguration => {
            "Not loaded: activeConfiguration in settings.json is not a safe file name"
        }
        Msg::Reload => "Reload",
        Msg::Reloaded => "Reloaded the file.",
        Msg::ReloadFailed => "Failed to reload the file",
        Msg::ReloadConfirm => "Unsaved changes will be discarded. Reload the file?",
        Msg::DiscardAndReload => "Discard and reload",
        Msg::CloseConfirm => "You have unsaved changes. Save them before closing?",
        Msg::DontSave => "Don't save",
        Msg::Cancel => "Cancel",
        Msg::SignalEventNotFound => "PowerToys Keyboard Manager does not seem to be running",
        Msg::SettingsLoadFailed => "Not loaded: failed to read settings.json",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_has_all_messages() {
        for &m in Msg::ALL {
            assert!(!tr(Lang::En, m).is_empty(), "{m:?}");
        }
    }
}
