# Strands Agents Concepts 学習ノート

[Strands Agents](https://strandsagents.com/) の **Concepts** 章を読み、汎用 AI エージェントの設計を学ぶためのノートです。

> ℹ️ **Strands Agents 自体は本プロジェクトの実装には使いません。**
> あくまで「よく整理されたエージェント設計の参考資料」として読み解き、
> 本プロジェクト（Rust）の設計方針（型安全な状態遷移 / SOLID / Testable / 関数型）へどう翻訳するかを各ノートの末尾にまとめています。

## 出典

- Agent Loop — <https://strandsagents.com/docs/user-guide/concepts/agents/agent-loop/>
- Conversation Management — <https://strandsagents.com/docs/user-guide/concepts/agents/conversation-management/>
- State Management — <https://strandsagents.com/docs/user-guide/concepts/agents/state/>
- Hooks — <https://strandsagents.com/docs/user-guide/concepts/agents/hooks/>
- Tool Executors — <https://strandsagents.com/docs/user-guide/concepts/tools/executors/>

（要約は 2026-05-31 時点の内容に基づきます。原典が一次情報です。）

## カバー範囲

Concepts 章のうち、**ループ設計の中核に直結する 5 ページ**を読み解きました。
残りの Interrupts / Retry Strategies / Multi-Agent / Session Management / Plugins は、
中核を実装したあとに必要に応じて追補する想定です（現時点では未収録）。

## 目次

| ノート | テーマ | 本プロジェクトとの関係 |
| --- | --- | --- |
| [01. Agent Loop](./01-agent-loop.md) | 推論 → ツール選択 → 実行 → フィードバックの再帰ループ、停止理由、ライフサイクル | **最重要**。内側／外側ループの設計の土台 |
| [02. Conversation Management](./02-conversation-management.md) | コンテキストウィンドウの維持戦略（スライディングウィンドウ / 要約 / null） | `ConversationHistory` をどう抽象化するか |
| [03. State Management](./03-state-management.md) | 会話履歴 / エージェント状態 / 呼び出し状態の 3 種類の状態 | 状態の分類と、モデル文脈に載せる／載せないの線引き |
| [04. Hooks](./04-hooks.md) | ライフサイクルイベントによる観測・介入の仕組み | 横断的関心事（ログ・権限・リトライ）を core から分離する |
| [05. Tool Execution](./05-tool-execution.md) | ツールレジストリ、逐次／並行実行、結果整形、エラー処理 | ツール実行ループの具体設計 |

## 全体像（このノート群から得た設計の核）

```
            ┌──────────────── 対話ループ（外側） ────────────────┐
            │                                                     │
 ユーザー入力 ─▶ ConversationHistory ─▶ ┌── ツール実行ループ（内側）──┐ ─▶ 最終応答 ─▶
            │                          │  推論(LLM)                  │      │
            │                          │    │                       │      │
            │                          │    ├─ end_turn ───────────▶ 終了   │
            │                          │    └─ tool_use             │      │
            │                          │         │                  │      │
            │                          │   ツール実行 ─▶ 結果を履歴へ ─┘      │
            │                          └────────────────────────────┘      │
            └─────────────────────────────────────────────────────────────┘
                    ▲ Hooks が各境界（呼び出し/モデル/ツール）でイベント発火
```

> 📌 この 2 重ループは**出発点の仮説**です。実際には計画・検証・リトライ・マルチエージェント委譲などで
> **3 重・4 重と入れ子が深くなりうる**。「ループが何重か」を固定するのではなく、各層を型安全な状態機械として表し、
> **入れ子を合成できる構造**にすること自体を学びの主眼とします。

3 つの学びの軸:

1. **ループは「停止理由（stop reason）」で駆動される。** ツール実行は「停止理由が `tool_use` のとき継続する」だけ。状態遷移を `enum` で表すと自然に対応づく。
2. **状態には階層がある。** モデルに渡る会話履歴と、渡らないアプリ状態／呼び出し状態を分けることで、文脈汚染とテスト容易性を両立できる。
3. **横断的関心事はイベント（Hooks）で外付けする。** core ループを汚さずに、ログ・権限・リトライ・キャンセルを足せる。
