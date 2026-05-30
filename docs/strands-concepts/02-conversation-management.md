# 02. Conversation Management

> 出典: <https://strandsagents.com/docs/user-guide/concepts/agents/conversation-management/>

## 目的

会話が伸びるにつれ生じる問題 ——**トークン上限・処理性能・関連性の劣化・一貫性の維持**—— を扱うのが
conversation manager。ユーザーメッセージ・エージェント応答・ツール使用と結果・システムプロンプトを、
**差し替え可能な戦略（pluggable strategy）** で管理する。

## 組み込み戦略

| 戦略 | 振る舞い |
| --- | --- |
| **SlidingWindow**（既定） | 直近 N 件を保持し、超過したら古いものから削除。ツール結果は head/tail を残し `<truncated chars="N"/>` で切り詰め、巨大メディアはプレースホルダ化 |
| **Summarizing** | 履歴を捨てずに**要約**して文脈を保つ。「主要トピック・使用ツール・技術情報」に絞った箇条書きの構造化要約。要約用エージェント/プロンプトを差し替え可能 |
| **Null** | 一切改変しない。デバッグや、上限を超えない短い会話向け |

## コンテキスト圧縮の 2 モード

- **Reactive（事後）**: オーバーフローでモデルがリクエストを拒否した後に縮小する。
- **Proactive（事前）**: モデル呼び出し前に、推定入力トークンがコンテキストウィンドウの閾値（既定 70%）を超えたら圧縮する。

トークン推定は、過去の使用メタデータ → モデルの `countTokens()` → 文字数ヒューリスティックの順にフォールバック。

## 主要な抽象（`ConversationManager` インターフェース）

- `apply_management()`: イベントサイクル後に履歴を整える
- `reduce_context()`: オーバーフロー時の縮小戦略を実装
- `removed_message_count`: 削除済み件数（セッション保存の効率化に使用）
- `register_hooks()`（任意）: proactive パターンを Hooks 経由で実現

カスタム実装はこのインターフェースを拡張してドメイン固有のロジックを定義する。

---

## 本プロジェクト（Rust）への設計示唆

### SOLID / 戦略の差し替え
- `ConversationHistory` の**整え方**を `trait ConversationManager` として切り出す（戦略パターン = OCP/DIP）。
  core ループは「毎ステップ後に manager を通す」ことだけ知り、戦略の中身は知らない。

```rust
trait ConversationManager {
    /// イベントサイクル後に履歴を整える（不変入力 → 新しい履歴）
    fn apply(&self, history: ConversationHistory) -> ConversationHistory;
    /// オーバーフロー時の縮小
    fn reduce(&self, history: ConversationHistory) -> Result<ConversationHistory, ContextError>;
}

struct SlidingWindow { max_messages: usize }
struct Summarizing<M: LanguageModel> { summarizer: M }
struct NullManager;
```

### 型安全
- 切り詰めマーカー（`<truncated chars="N"/>`）やメディアのプレースホルダ化は、メッセージ内容を `enum MessageContent { Text, ToolResult, Media, Truncated { chars: usize }, .. }` で表せば失われた情報も型に残る。

### Testable（副作用の追い出し）
- `apply` / `reduce` を**純粋関数**（入力履歴 → 出力履歴）にすれば、トークン上限の挙動を入力データだけで検証できる。
- トークン推定は副作用（モデルの `countTokens`）を含みうるので、`trait TokenCounter` として分離し、テストでは固定値を返すフェイクを使う。

### 関数型のエッセンス
- 履歴は**不変**に扱い、戦略は「履歴 → 新しい履歴」の変換関数として合成する（Null = 恒等関数、と捉えると綺麗）。
- proactive 圧縮は「閾値超過なら縮小、さもなくば素通し」という純粋な条件分岐に落ちる。
