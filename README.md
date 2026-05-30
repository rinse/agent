# agent

汎用 AI エージェントを **設計の練習台** として一から実装する、学習用プロジェクトです。
Claude Code / Codex / Gemini のような「ツールを実行し、外部と接続し、対話をループで回す」エージェントの中核を、
型安全で Testable な形に落とし込むことを目的にしています。

> ⚠️ これは学習用プロジェクトです。コーディング作業そのものには既存のエージェント（Claude Code 等）を使い、
> 本リポジトリではその**仕組みを自分で組み立てて理解する**ことを狙いとします。

---

## 何を作るか

- ユーザーと対話し、必要に応じて **ツールを実行する** 汎用エージェント
- **外部との接続**（HTTP / プロセス起動など）を抽象化したアダプタ
- **MCP（Model Context Protocol）** クライアントとしてのツール連携

機能の網羅性よりも、**ループとその状態遷移をどう設計するか** に主眼を置きます。

### スコープ

| やること | やらないこと |
| --- | --- |
| 対話ループ / ツール実行ループの実装 | 特定 LLM ベンダー機能の作り込み |
| 型安全な状態遷移の設計 | リッチな UI / TUI の作り込み |
| MCP・外部接続の抽象化 | プロダクション運用（認証基盤・課金など） |
| 副作用を境界に追い出した Testable な構造 | 高度な最適化・分散実行 |

---

## 学びの中心：（おそらく）入れ子のループ

このプロジェクトで一番学びたいのが **ループ処理** です。ここでは、もっとも分かりやすい形として
「対話ループ（外側）」と「ツール実行ループ（内側）」の **2 重ループ** を出発点に置きます。

> 📌 **これは確定した構造ではなく、現時点での仮説です。**
> 「2 つのループ」というのは、いま私（実装者）に見えている範囲でエージェントループの核心はこれだろうと
> 想像しているもの、という意味です。実際には設計を進めるうちに、
> 計画／検証のサブループ、リトライやエラー回復のループ、マルチエージェントの委譲など、
> **3 重・4 重と層が増えていく可能性があります**。
> 「いくつのループか」を先に決めるのではなく、各ループが**型安全な状態遷移**として表現でき、
> 必要に応じて**入れ子を増やせる構造**になっていること自体を、この学習の主眼とします。

### 1. 対話ループ（外側）

ユーザーとエージェントのやり取りを繰り返し処理します。各ターンの履歴は `ConversationHistory` に蓄積され、
次のモデル呼び出しの文脈になります。

```
ユーザー入力 ─▶ ConversationHistory へ追加 ─▶ エージェントのターン処理 ─▶ 応答
      ▲                                                                 │
      └─────────────────────────────────────────────────────────────────┘
```

### 2. ツール実行ループ（内側）

1 ターンの中で、モデルの応答に応じて「ツールを実行するか」を判断し、結果を履歴へ戻して再びモデルに問い合わせます。
最終応答が返るまでこのループを回します。

```
モデルへ問い合わせ
   │
   ├─ 最終応答         ─▶ ターン終了（ユーザーへ返す）
   │
   └─ ツール実行要求   ─▶ ツール実行 ─▶ 結果を ConversationHistory へ ─┐
                                                                       │
       ▲───────────────────────────────────────────────────────────────┘
```

この「判断 → 実行 → 結果処理」の遷移を、**条件分岐の積み重ねではなく型で表現する** のが本プロジェクトの肝です。
そして、ここにさらに別のループ（計画・検証・リトライなど）が入れ子で重なっても破綻しないよう、
**ループを合成可能な状態機械として設計できるか** を学びます。

---

## 設計方針

### 型安全な状態遷移

ターンの状態を `enum`（直和型 / ADT）で表現し、ありえない状態を**コンパイル時に排除**します。
`match` の網羅性チェックにより、遷移の漏れを型システムに検出させます。

```rust
/// 1 ターンの中でモデルが返す結果 = なぜ生成が止まったか（stop reason）。
/// ループの継続/終了はこの値だけで決まる。
enum StopReason {
    EndTurn(AssistantMessage),     // 通常完了 → ユーザーへ返す
    ToolUse(Vec<ToolCall>),        // ツール実行要求 → ループ継続
    MaxTokens,                     // 上限到達（回復不能）→ エラー
    Cancelled,                     // 外部キャンセル
}

/// ツール実行ループの状態
enum Turn {
    AwaitingModel,                 // モデルの応答待ち
    ExecutingTools(Vec<ToolCall>), // ツール実行中
    Completed(AssistantMessage),   // ターン完了
}
```

> ループが「停止理由（stop reason）」で駆動されるという捉え方は、参考資料の
> [Agent Loop ノート](./docs/strands-concepts/01-agent-loop.md) から得たものです。

### SOLID を意識したモジュール分割

外部に依存する操作は **trait（ポート）** として定義し、実装（アダプタ）を差し替え可能にします。
これにより依存性逆転（DIP）と単一責任（SRP）を満たします。

```rust
/// LLM への問い合わせ（ポート）
trait LanguageModel {
    async fn complete(&self, history: &ConversationHistory) -> Result<ModelResponse>;
}

/// 実行可能なツール（ポート）
trait Tool {
    fn name(&self) -> &str;
    async fn invoke(&self, input: ToolInput) -> Result<ToolOutput>;
}

/// ツールの集合を解決する（MCP・ローカルツールを同一視）
trait ToolRegistry {
    fn resolve(&self, name: &str) -> Option<&dyn Tool>;
}
```

> ⚠️ 上は説明用の簡略形。`async fn` をトレイトに持たせると dyn 非互換になり `&dyn Tool` を作れません。
> 実装では `#[async_trait]` / 明示的な `Pin<Box<dyn Future>>` / enum ディスパッチのいずれかを選びます。
> 詳細は [05. Tool Execution ノート](./docs/strands-concepts/05-tool-execution.md)。

### 副作用を追い出した Testable な設計

ループを駆動する**コア（純粋なロジック）** と、**副作用（モデル呼び出し・ツール実行・I/O）** を分離します。
副作用はすべてポート越しに注入されるため、テストではフェイク実装を渡すだけで**ループ全体を決定的に検証**できます。

```
[ pure core: 状態遷移の決定 ]  ←  trait 経由で注入  ←  [ adapters: LLM / MCP / プロセス ]
        ↑ ここはテストで完全に再現可能
```

### 関数型のエッセンス

- 状態を**不変**に扱い、遷移は「新しい状態を返す」関数として記述する
- エラーは例外ではなく `Result` で値として扱い、フローを明示する
- ループ駆動部は **状態 + 入力 → 次の状態 + 作用** の純粋関数に寄せる

---

## アーキテクチャ概観

```
┌─────────────────────────────────────────────┐
│                 Agent (driver)              │
│   対話ループ / ツール実行ループの状態遷移   │  ← pure core
└───────────────┬─────────────────────────────┘
                │ ports (traits)
   ┌────────────┼─────────────┬──────────────┐
   ▼            ▼             ▼              ▼
LanguageModel   Tool      ToolRegistry    McpClient   ← adapters (副作用)
```

| レイヤ | 責務 |
| --- | --- |
| **core** | `ConversationHistory`・状態遷移・ループ駆動（副作用なし） |
| **ports** | `LanguageModel` / `Tool` / `ToolRegistry` などの trait 定義 |
| **adapters** | LLM クライアント・MCP クライアント・プロセス実行などの具体実装 |

---

## ディレクトリ構成（予定）

Rust の Cargo workspace を想定しています。

```
agent/
├── Cargo.toml            # workspace 定義
├── crates/
│   ├── core/             # ループ・状態遷移・ConversationHistory（純粋）
│   ├── ports/            # trait 定義（LanguageModel, Tool, ...）
│   ├── adapters/         # LLM / MCP / プロセス実行などの実装
│   └── cli/              # エントリポイント（対話 UI）
├── docs/
│   └── strands-concepts/ # 設計ノート（Strands Concepts の読み解き）
└── README.md
```

> 構成は実装の進行に合わせて見直します。

---

## 開発

### 必要環境

- Rust（stable, edition 2021 以降）

### よく使うコマンド

```bash
cargo build          # ビルド
cargo test           # テスト（コアの状態遷移はフェイクで決定的に検証）
cargo clippy         # Lint
cargo fmt            # フォーマット
cargo run -p cli     # 対話エージェントの起動
```

---

## 設計の参考資料

エージェント設計の学習にあたり、[Strands Agents](https://strandsagents.com/) の **Concepts** 章を読み解き、
本プロジェクトの方針（型安全 / SOLID / Testable / 関数型）へ翻訳したノートを [`docs/strands-concepts/`](./docs/strands-concepts/) にまとめています。
**Strands Agents 自体は実装には使いません**（あくまで参考資料）。

- [00. インデックス](./docs/strands-concepts/README.md)
- [01. Agent Loop](./docs/strands-concepts/01-agent-loop.md) — 推論→ツール→実行→フィードバックの再帰ループ、停止理由
- [02. Conversation Management](./docs/strands-concepts/02-conversation-management.md) — コンテキスト維持戦略
- [03. State Management](./docs/strands-concepts/03-state-management.md) — 3 種類の状態
- [04. Hooks](./docs/strands-concepts/04-hooks.md) — ライフサイクルイベントによる拡張
- [05. Tool Execution](./docs/strands-concepts/05-tool-execution.md) — レジストリ・逐次/並行実行・エラー処理

---

## ロードマップ

- [x] `ConversationHistory` と基本の型（メッセージ・ツール呼び出し）
- [ ] ツール実行ループの状態遷移（`Turn` ベース）
- [ ] 対話ループの駆動と `LanguageModel` ポート
- [ ] ローカルツールの実装と `ToolRegistry`
- [ ] MCP クライアントアダプタ
- [ ] フェイク実装によるループ全体のテスト

---

## ライセンス

学習用プロジェクトのため未定。
