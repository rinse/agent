# 01. Agent Loop

> 出典: <https://strandsagents.com/docs/user-guide/concepts/agents/agent-loop/>

## 中核となる考え方

> *"A language model can answer questions. An agent can do things."*

言語モデルは質問に答えるだけだが、**エージェントは行動できる**。エージェントループとは、
言語モデルに「行動する能力」を足して自律的にするもの。**推論（reasoning）と行動（action）のサイクル**を
回すことで、外部情報へのアクセスを伴う複数ステップの問題解決が可能になる。

## ループのアーキテクチャ

基本サイクルは 4 つのステージから成る。

1. **推論（LLM）** — モデルが文脈を処理し、次の行動を決める
2. **ツール選択** — モデルがどのツールを（あるいは何も）使うか決める
3. **ツール実行** — システムが検証・解決・実行する
4. **フィードバックループ** — 結果を会話履歴へ追加し、再び推論へ戻る

この**再帰構造**が反復ごとに文脈を蓄積する。各ツール結果がモデルへ戻ることで、
ステップ間の手動介入なしに多段推論が成立する。

## ライフサイクルのフェーズ

### Starting（開始）
ツールを登録し、conversation manager を構成し、**ユーザー入力を会話履歴の最初のメッセージとして配置**して初期化する。

### Stop Reasons（停止理由 / ループの終了条件）
ループの継続・終了は「モデルがなぜ止まったか」で決まる。これが設計上もっとも重要。

| 停止理由 | 意味 | ループ |
| --- | --- | --- |
| `end_turn` | 通常完了。モデルが応答し終えた | **終了** |
| `tool_use` | 1 つ以上のツール実行を要求 | **継続** |
| `cancelled` | `agent.cancel()` 等による外部キャンセル | 終了 |
| `max_tokens` | トークン上限で応答が切れた（回復不能） | 終了（エラー） |
| `stop_sequence` | 設定した停止シーケンスに到達 | 終了 |
| `content_filtered` / `guardrail` | 安全機構の作動 | 終了 |

### Extending（拡張点）
呼び出し前後・モデル呼び出し前後・ツール実行前後といった**チェックポイントでライフサイクルイベントを発火**する。
これにより Hooks 経由で観測・改変ができる（→ [04. Hooks](./04-hooks.md)）。

## メッセージと会話の扱い

- **User メッセージ**: 初回リクエストやフォローアップ。テキスト・ツール結果・メディアを含む。
- **Assistant メッセージ**: モデル出力。テキスト応答・ツール要求・推論トレース。

蓄積された履歴は conversation manager がコンテキストウィンドウ内に維持し、**モデルの作業記憶**として機能する
（→ [02. Conversation Management](./02-conversation-management.md)）。

## ツール実行フロー

実行システムは:

- リクエストをツールスキーマに照らして**検証**
- レジストリからツールを**解決**
- エラーハンドリング付きで**実行**
- 結果をメッセージとして**整形**

**重要**: ツール失敗はループを止めず、**エラー結果としてモデルへ返す**。これによりモデルが回復を試みられる。

## キャンセル

- **内部**: `agent.cancel()` がシグナルを立て、モデルストリーミング中やツール実行前のチェックポイントで確認される。
- **外部**: `invoke()` / `stream()` の `cancelSignal` に `AbortSignal` を渡す。
- **ツール協調**: 実行開始後は協調的キャンセル。ツールが `cancelSignal.aborted` をポーリングするか API へ転送する。
- 呼び出し完了後にシグナルは自動クリアされ、エージェントは即再利用可能になる。

## よくある課題

| 問題 | 対策 |
| --- | --- |
| コンテキストウィンドウ枯渇 | ツール出力を簡潔化 / スキーマ簡素化 / conversation manager 戦略 / サブタスク分解 |
| 不適切なツール選択 | ツール説明を明確化して曖昧さを除く |
| `MaxTokensReached` | 文脈縮小 / トークン上限引き上げ / タスク分割 |

---

## 本プロジェクト（Rust）への設計示唆

### 型安全な状態遷移
- **停止理由（stop reason）を `enum` の中心に据える。** ループ継続/終了は `match` の網羅で表現でき、分岐漏れをコンパイラが検出する。

```rust
enum StopReason {
    EndTurn(AssistantMessage),
    ToolUse(Vec<ToolCall>),
    Cancelled,
    MaxTokens,        // 回復不能 → エラーとして扱う
    StopSequence,
    Filtered,
}

/// ループ 1 ステップの遷移：状態 + 副作用結果 → 次の状態（純粋関数）
fn step(history: &ConversationHistory, response: StopReason) -> Transition {
    match response {
        StopReason::EndTurn(msg)   => Transition::Done(msg),
        StopReason::ToolUse(calls) => Transition::RunTools(calls),
        StopReason::MaxTokens      => Transition::Fail(LoopError::ContextExhausted),
        _                          => Transition::Done(/* … */),
    }
}
```

### SOLID
- 「推論」「ツール解決」「ツール実行」をそれぞれ別 trait（ポート）に切る → SRP / DIP。
- ループ駆動部は具体実装に依存せず trait のみに依存する。

### Testable（副作用の追い出し）
- `step` のような**遷移決定を純粋関数**にし、LLM 呼び出し・ツール実行は呼び出し側（ドライバ）が trait 経由で行う。
- フェイクの `LanguageModel`（停止理由の列を返すだけ）でループ全体を決定的にテストできる。

### 関数型のエッセンス
- ツール失敗を例外でなく `Result`/結果メッセージとして**値で扱い**、ループを止めずモデルへ戻す設計と相性が良い。
- 状態は不変、遷移は「新しい状態を返す」関数で表す。
