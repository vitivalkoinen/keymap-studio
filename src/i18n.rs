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
    LegendRemapSource,
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
}

impl Msg {
    /// すべての値(テスト用)。
    #[cfg(test)]
    pub const ALL: &[Msg] = &[
        Msg::ConfigPathNotFound,
        Msg::ConfigLoadFailed,
        Msg::KeyboardHeading,
        Msg::LegendRemapSource,
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
        Msg::LegendRemapSource => "Highlighted keys are remapped.",
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
