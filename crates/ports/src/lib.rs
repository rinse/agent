//! ポートの再エクスポート。
//!
//! ポート trait の定義は `agent-core/src/ports.rs` に置き、循環依存を回避している。
//! このクレートはそれらを再エクスポートする薄いラッパ。
//! ユーザーは `agent_core::ports::LanguageModel` でも `ports::LanguageModel` でも使える。

pub use agent_core::ports::*;
