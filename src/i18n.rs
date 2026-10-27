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
    InitialBackupCreated,
    InitialBackupFailed,
    DataDirNotFound,
    RestoreButton,
    RestoreUnavailableArgument,
    RestoreUnavailableNoBackup,
    RestoreUnavailableNoPath,
    RestoreUnavailableOtherConfiguration,
    RestoreConfirm,
    RestoreDefinitionFile,
    RestoreBackedUpAt,
    RestoreNoDefinition,
    RestoreDiscardsUnsaved,
    Restore,
    Restored,
    RestoredNotApplied,
    RestoredReloadFailed,
    RestoreBackupUnreadable,
    RestoreAbortedLocked,
    RestoreFailedBackup,
    RestoreFailed,
    InitialBackupSkippedByArgument,
    SaveBlockedByInitialBackup,
    PowerToysNotInstalled,
    KeyboardManagerNotConfigured,
    Recheck,
    SearchKeys,
    NoMatchingKeys,
    PickOnKeyboard,
    PickingOnKeyboard,
    PickBanner,
    PickBannerHint,
    CategorySpecial,
    CategoryLetters,
    CategoryDigits,
    CategorySymbols,
    CategoryFunction,
    CategoryModifiers,
    CategoryEditing,
    CategoryNavigation,
    CategoryNumpad,
    CategoryMediaAndOther,
    KeymapsHeading,
    SaveAsNewKeymap,
    ImportKeymap,
    NoKeymaps,
    Apply,
    LoadIntoEditor,
    OverwriteKeymap,
    RenameKeymap,
    DeleteKeymap,
    ExportKeymap,
    InUse,
    InUseHint,
    UnreadableKeymaps,
    PresetsHeading,
    PresetOnishi,
    ReadOnlyPreset,
    OverlayPreset,
    PresetShortcutNote,
    NoConfigLoaded,
    NewKeymapPrompt,
    NewKeymapIncludesUnsaved,
    RenamePrompt,
    KeymapNameEmpty,
    KeymapNameTaken,
    DeleteKeymapConfirm,
    OverwriteKeymapConfirm,
    ApplyDiscardConfirm,
    LoadDiscardConfirm,
    KeymapFileFilter,
    KeymapSaved,
    KeymapOverwritten,
    KeymapRenamed,
    KeymapDeleted,
    KeymapExported,
    KeymapImported,
    KeymapFailed,
    ImportRejected,
    KeymapDirNotFound,
    Applied,
    LoadedIntoEditor,
    PresetOverlaid,
    KeymapInvalidMalformed,
    KeymapInvalidFormat,
    KeymapInvalidVersion,
    KeymapInvalidLayout,
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
        Msg::InitialBackupCreated,
        Msg::InitialBackupFailed,
        Msg::DataDirNotFound,
        Msg::RestoreButton,
        Msg::RestoreUnavailableArgument,
        Msg::RestoreUnavailableNoBackup,
        Msg::RestoreUnavailableNoPath,
        Msg::RestoreUnavailableOtherConfiguration,
        Msg::RestoreConfirm,
        Msg::RestoreDefinitionFile,
        Msg::RestoreBackedUpAt,
        Msg::RestoreNoDefinition,
        Msg::RestoreDiscardsUnsaved,
        Msg::Restore,
        Msg::Restored,
        Msg::RestoredNotApplied,
        Msg::RestoredReloadFailed,
        Msg::RestoreBackupUnreadable,
        Msg::RestoreAbortedLocked,
        Msg::RestoreFailedBackup,
        Msg::RestoreFailed,
        Msg::InitialBackupSkippedByArgument,
        Msg::SaveBlockedByInitialBackup,
        Msg::PowerToysNotInstalled,
        Msg::KeyboardManagerNotConfigured,
        Msg::Recheck,
        Msg::SearchKeys,
        Msg::NoMatchingKeys,
        Msg::PickOnKeyboard,
        Msg::PickingOnKeyboard,
        Msg::PickBanner,
        Msg::PickBannerHint,
        Msg::CategorySpecial,
        Msg::CategoryLetters,
        Msg::CategoryDigits,
        Msg::CategorySymbols,
        Msg::CategoryFunction,
        Msg::CategoryModifiers,
        Msg::CategoryEditing,
        Msg::CategoryNavigation,
        Msg::CategoryNumpad,
        Msg::CategoryMediaAndOther,
        Msg::KeymapsHeading,
        Msg::SaveAsNewKeymap,
        Msg::ImportKeymap,
        Msg::NoKeymaps,
        Msg::Apply,
        Msg::LoadIntoEditor,
        Msg::OverwriteKeymap,
        Msg::RenameKeymap,
        Msg::DeleteKeymap,
        Msg::ExportKeymap,
        Msg::InUse,
        Msg::InUseHint,
        Msg::UnreadableKeymaps,
        Msg::PresetsHeading,
        Msg::PresetOnishi,
        Msg::ReadOnlyPreset,
        Msg::OverlayPreset,
        Msg::PresetShortcutNote,
        Msg::NoConfigLoaded,
        Msg::NewKeymapPrompt,
        Msg::NewKeymapIncludesUnsaved,
        Msg::RenamePrompt,
        Msg::KeymapNameEmpty,
        Msg::KeymapNameTaken,
        Msg::DeleteKeymapConfirm,
        Msg::OverwriteKeymapConfirm,
        Msg::ApplyDiscardConfirm,
        Msg::LoadDiscardConfirm,
        Msg::KeymapFileFilter,
        Msg::KeymapSaved,
        Msg::KeymapOverwritten,
        Msg::KeymapRenamed,
        Msg::KeymapDeleted,
        Msg::KeymapExported,
        Msg::KeymapImported,
        Msg::KeymapFailed,
        Msg::ImportRejected,
        Msg::KeymapDirNotFound,
        Msg::Applied,
        Msg::LoadedIntoEditor,
        Msg::PresetOverlaid,
        Msg::KeymapInvalidMalformed,
        Msg::KeymapInvalidFormat,
        Msg::KeymapInvalidVersion,
        Msg::KeymapInvalidLayout,
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
        Msg::InitialBackupCreated => {
            "Backed up the PowerToys Keyboard Manager settings (first launch)"
        }
        Msg::InitialBackupFailed => {
            "Failed to back up the PowerToys Keyboard Manager settings at first launch (will try again at next launch)"
        }
        Msg::DataDirNotFound => "the data folder (next to the executable) could not be determined",
        Msg::RestoreButton => "Restore settings from before installation…",
        Msg::RestoreUnavailableArgument => {
            "Not available when a file is specified on the command line."
        }
        Msg::RestoreUnavailableNoBackup => {
            "Not available: there is no backup from the first launch."
        }
        Msg::RestoreUnavailableNoPath => {
            "Not available: the config file path could not be determined."
        }
        Msg::RestoreUnavailableOtherConfiguration => {
            "Not available: the active configuration is different from the one backed up at the first launch"
        }
        Msg::RestoreConfirm => {
            "The remap definition will be restored to the state backed up at the first launch of Keymap Studio. The current remaps will be lost (a copy of the current file is kept in the backups folder). settings.json and editorSettings.json are not changed."
        }
        Msg::RestoreDefinitionFile => "Remap definition file",
        Msg::RestoreBackedUpAt => "Backed up at",
        Msg::RestoreNoDefinition => {
            "The remap definition file did not exist at that time, so an empty definition (no remaps) will be written."
        }
        Msg::RestoreDiscardsUnsaved => "Your unsaved changes will also be discarded.",
        Msg::Restore => "Restore",
        Msg::Restored => "Restored the remap definition from before installation.",
        Msg::RestoredNotApplied => {
            "Restored, but PowerToys may not have applied the changes (turn Keyboard Manager off and on in PowerToys to apply them)"
        }
        Msg::RestoredReloadFailed => "Restored, but failed to reload the file",
        Msg::RestoreBackupUnreadable => {
            "Not restored: failed to read the backup from the first launch"
        }
        Msg::RestoreAbortedLocked => {
            "Not restored: the file is locked by another program (timed out)"
        }
        Msg::RestoreFailedBackup => "Not restored: failed to create a backup of the current file",
        Msg::RestoreFailed => "Failed to restore",
        Msg::InitialBackupSkippedByArgument => {
            "A file was specified on the command line, so the backup of the settings from before installation is neither created nor restored."
        }
        Msg::SaveBlockedByInitialBackup => {
            "Saving is disabled until the backup of the settings from before installation has been created (it will be tried again at next launch)."
        }
        Msg::PowerToysNotInstalled => {
            "PowerToys was not found. Install PowerToys first (see the official installation page below), then press Check again."
        }
        Msg::KeyboardManagerNotConfigured => {
            "PowerToys Keyboard Manager settings were not found. Turn on Keyboard Manager once in the PowerToys settings, then press Check again."
        }
        Msg::Recheck => "Check again",
        Msg::SearchKeys => "Search keys",
        Msg::NoMatchingKeys => "No matching keys",
        Msg::PickOnKeyboard => "Pick on keyboard",
        Msg::PickingOnKeyboard => "Picking on the keyboard…",
        Msg::PickBanner => "Click a key on the keyboard to choose the remap target for",
        Msg::PickBannerHint => {
            "Keys with a highlighted border can be chosen; dimmed keys cannot. Press Esc or Cancel to stop."
        }
        Msg::CategorySpecial => "Special (Disable, Win)",
        Msg::CategoryLetters => "Letters",
        Msg::CategoryDigits => "Digits",
        Msg::CategorySymbols => "Symbols",
        Msg::CategoryFunction => "Function keys",
        Msg::CategoryModifiers => "Modifiers",
        Msg::CategoryEditing => "Enter, Tab, Space, Backspace, Esc",
        Msg::CategoryNavigation => "Navigation",
        Msg::CategoryNumpad => "Numpad",
        Msg::CategoryMediaAndOther => "Media and other",
        Msg::KeymapsHeading => "Keymaps",
        Msg::SaveAsNewKeymap => "Save current as new keymap…",
        Msg::ImportKeymap => "Import…",
        Msg::NoKeymaps => "No saved keymaps yet.",
        Msg::Apply => "Apply",
        Msg::LoadIntoEditor => "Edit",
        Msg::OverwriteKeymap => "Overwrite",
        Msg::RenameKeymap => "Rename",
        Msg::DeleteKeymap => "Delete",
        Msg::ExportKeymap => "Export…",
        Msg::InUse => "In use",
        Msg::InUseHint => "The remaps currently saved in PowerToys match this one.",
        Msg::UnreadableKeymaps => "Some files in the keymaps folder could not be read:",
        Msg::PresetsHeading => "Presets",
        Msg::PresetOnishi => "Onishi layout",
        Msg::ReadOnlyPreset => "(built-in, read-only)",
        Msg::OverlayPreset => "Add to current edits",
        Msg::PresetShortcutNote => {
            "Note: key remaps also affect shortcuts that use them. For example, with the Onishi layout, pressing Ctrl with the physical S key sends Ctrl+I."
        }
        Msg::NoConfigLoaded => "Not available: no PowerToys remap settings are loaded.",
        Msg::NewKeymapPrompt => {
            "Name of the new keymap (saves the remaps currently shown on the keyboard):"
        }
        Msg::NewKeymapIncludesUnsaved => "Unsaved changes are included.",
        Msg::RenamePrompt => "New name of the keymap:",
        Msg::KeymapNameEmpty => "Enter a name.",
        Msg::KeymapNameTaken => "A keymap with this name already exists.",
        Msg::DeleteKeymapConfirm => "Delete this keymap? This cannot be undone.",
        Msg::OverwriteKeymapConfirm => {
            "Overwrite this keymap with the remaps currently shown on the keyboard?"
        }
        Msg::ApplyDiscardConfirm => {
            "Applying replaces all remaps in PowerToys with this one and takes effect immediately. Your unsaved changes will be discarded."
        }
        Msg::LoadDiscardConfirm => {
            "Loading this into the editor discards your unsaved changes. Nothing is saved until you press Save."
        }
        Msg::KeymapFileFilter => "Keymap Studio keymap",
        Msg::KeymapSaved => "Saved the keymap",
        Msg::KeymapOverwritten => "Overwrote the keymap",
        Msg::KeymapRenamed => "Renamed the keymap",
        Msg::KeymapDeleted => "Deleted the keymap",
        Msg::KeymapExported => "Exported the keymap",
        Msg::KeymapImported => "Imported the keymap",
        Msg::KeymapFailed => "Keymap operation failed",
        Msg::ImportRejected => "Not imported",
        Msg::KeymapDirNotFound => {
            "The keymaps folder (next to the executable) could not be determined."
        }
        Msg::Applied => "Applied",
        Msg::LoadedIntoEditor => "Loaded into the editor (not saved yet; press Save to apply)",
        Msg::PresetOverlaid => {
            "Added the preset to the current edits (not saved yet; press Save to apply)"
        }
        Msg::KeymapInvalidMalformed => "not a valid keymap file",
        Msg::KeymapInvalidFormat => "not a Keymap Studio keymap",
        Msg::KeymapInvalidVersion => "unsupported formatVersion",
        Msg::KeymapInvalidLayout => "unsupported keyboard layout",
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
