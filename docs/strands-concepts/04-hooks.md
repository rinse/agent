# 04. Hooks

> 出典: <https://strandsagents.com/docs/user-guide/concepts/agents/hooks/>

## 概要

Hooks は、エージェントの実行を通じて振る舞いを**観測・改変**できる、合成可能なイベント購読の仕組み。
ライフサイクルの離散的な段階で発火する「強く型付けされたイベントコールバック（strongly-typed event callbacks）」を提供し、
組み込みコンポーネントとユーザーコードの双方がエージェント動作に介入できる。

## ライフサイクルイベント

### 単一エージェント
- **呼び出し境界**: `BeforeInvocationEvent` / `AfterInvocationEvent` がリクエスト全体を挟む
- **モデル推論**: `BeforeModelCallEvent` / `AfterModelCallEvent` が LLM 呼び出しを挟む
- **ツール実行**: `BeforeToolCallEvent` / `AfterToolCallEvent` がツール呼び出しを挟む
- **履歴**: `MessageAddedEvent` が会話メッセージ記録時に発火

### マルチエージェント
- `BeforeMultiAgentInvocationEvent` / `AfterMultiAgentInvocationEvent`（オーケストレータ実行を括る）
- `BeforeNodeCallEvent` / `AfterNodeCallEvent`（個々のノード実行を括る）
- `MultiAgentHandoffEvent`（ノード間の遷移を通知）

## 登録とコールバックの順序

- `agent.add_hook(callback, EventType)` で登録。関数シグネチャから型推論も可能。
- 複数コールバックは Plugin としてまとめて束ねられる。
- **順序**: Before は登録順、After は逆順（クリーンアップ対称性）。`order` 数値で SDK 内部との相対位置を制御（`SDK_FIRST = -100`, `SDK_LAST = 100`）。

## 拡張のためのイベントプロパティ

イベントのフィールドを通じて振る舞いを書き換えられる:

- `BeforeToolCallEvent.selected_tool` — ツールを差し替え/横取り
- `AfterToolCallEvent.result` — ツール出力を書き換え
- `AfterInvocationEvent.resume` — フォローアップ呼び出しを誘発
- `BeforeToolCallEvent.cancel_tool` / `retry` — 実行フローを制御

> これにより「**ロギング・ツール結果フィルタ・権限強制・リトライ**を、core エージェントコードを変更せずに」実現できる。

---

## 本プロジェクト（Rust）への設計示唆

### SOLID（とくに OCP）
- Hooks は **Open-Closed 原則そのもの**。core ループを閉じたまま、横断的関心事（ログ・権限・リトライ・キャンセル）を外から足せる。
- 「ツール実行前に権限チェック」「ツール出力のフィルタ」といった機能を core の `if` で書かず、観測者として分離する。

```rust
/// ライフサイクルイベント（強く型付け）
enum LoopEvent<'a> {
    BeforeModelCall(&'a ConversationHistory),
    AfterModelCall(&'a StopReason),
    BeforeToolCall(&'a mut ToolCall),   // 差し替え/キャンセルを許す
    AfterToolCall(&'a mut ToolOutput),  // 結果の書き換えを許す
    MessageAdded(&'a Message),
}

trait Hook {
    fn on_event(&self, event: &mut LoopEvent<'_>) -> HookControl;
}

/// Before フックがフローを制御する手段
enum HookControl { Continue, CancelTool, Retry }
```

### 型安全な状態遷移
- フックの「介入」を `HookControl` のような `enum` で表せば、キャンセル/リトライ/続行の分岐が型に乗り、`match` で網羅される。
- Before は登録順・After は逆順という規約は、登録リストを保持して順序を明示管理すれば再現できる。

### Testable（副作用の追い出し）
- フックを使えば、テスト時に「呼ばれたイベントを記録するだけのスパイ・フック」を挿して、ループが正しい順序でイベントを発火するか検証できる。
- 権限・ロギングなどの副作用をフック側へ追い出すことで、core ループ自体は純粋に保てる。

### 関数型のエッセンス
- フックは本質的に「イベント → 制御」の関数。複数フックは合成（順に畳み込む）して 1 本のパイプラインにできる。
