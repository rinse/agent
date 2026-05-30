# 05. Tool Execution（Executors）

> 出典: <https://strandsagents.com/docs/user-guide/concepts/tools/executors/>

## 概要

Tool executor は、**1 つのアシスタントターン内の複数ツール呼び出しをどう実行するか**（並行 or 逐次）を決める。

## 実行モード

| モード | いつ使う | 特性 |
| --- | --- | --- |
| **Concurrent（既定）** | 依存のない複数ツール | すべて並列実行。総時間は「最も遅いツール」に一致し、レイテンシ最小 |
| **Sequential** | 後のツールが前の結果/副作用に依存する | モデルが指定した順に実行 |

## イベント順序と制御

- どちらのモードでも、**ツール単位のイベント順序は保たれる**:
  `BeforeToolCall` → ツールのストリーミングイベント → `AfterToolCall` → `ToolResult`。
- 並行モードでは**異なるツール間でイベントが交錯**しうるが、個々のツール内の順序は維持される。
- キャンセルは両モードで同一に機能（`agent.cancel()`）。起動前キャンセルは開始を防ぎ、実行中は協調的監視が必要。

## 現状の制限

- カスタム executor は未対応（将来対応予定。Python SDK の GitHub Issue #762 で追跡）。

## （Agent Loop より再掲）ツール実行システムの責務

- リクエストをツールスキーマで**検証**
- レジストリからツールを**解決**
- エラーハンドリング付きで**実行**
- 結果をメッセージへ**整形**
- ツール失敗は**エラー結果としてモデルへ返す**（ループは止めない）

---

## 本プロジェクト（Rust）への設計示唆

### SOLID
- ツールは `trait Tool`、集合解決は `trait ToolRegistry`、実行戦略は `trait ToolExecutor` に分離（SRP）。
  ローカルツールも MCP ツールも同じ `Tool` として扱えば、core から見て区別不要（LSP/DIP）。

```rust
trait Tool {
    fn name(&self) -> &str;
    fn schema(&self) -> &ToolSchema;
    async fn invoke(&self, input: ToolInput) -> ToolOutput; // 失敗も ToolOutput::Error で表現
}

trait ToolRegistry {
    fn resolve(&self, name: &str) -> Option<&dyn Tool>;
}

/// 1 ターン分の複数呼び出しをどう走らせるか
trait ToolExecutor {
    async fn run(&self, calls: Vec<ResolvedCall>) -> Vec<ToolOutput>;
}
struct Concurrent;  // join_all 相当
struct Sequential;  // 順次
```

> ⚠️ **`async fn` × 動的ディスパッチの注意点（Rust 設計の肝）**
> 上の `Tool` は説明用の簡略形。**`async fn` をトレイトに置くと、そのトレイトは dyn 非互換（dyn-incompatible）になり、
> `&dyn Tool` / `Option<&dyn Tool>` を作れません**（stable Rust。`async fn in trait` は 1.75 で安定化したが、戻り値が匿名の `impl Future` になるため trait object 化できない）。
> 「ローカルツールと MCP ツールを 1 本の `&dyn Tool` に統一する（LSP/DIP）」という狙いは、素直に書くとコンパイルが通らない。
> これはまさに本プロジェクトが向き合いたい「複雑さを型でどう扱うか」の典型で、選択肢は概ね次の 3 つ:
>
> 1. **`#[async_trait]`**（async-trait crate）— `invoke` を `Pin<Box<dyn Future>>` に脱糖し dyn 化可能にする。最も手軽だが毎回ヒープ確保。
> 2. **明示的に `fn invoke(&self, ..) -> Pin<Box<dyn Future<Output = ToolOutput> + '_>>`** を返す — マクロなしで同等。
> 3. **enum ディスパッチ**（`enum Tool { Local(..), Mcp(..) }`）— `dyn` を使わず静的に分岐。型が閉じている前提なら最も高速・素直。
>
> `LanguageModel` / `ToolExecutor` / `McpClient` / 要約用 `Summarizing<M>` など、**async と動的ディスパッチが交わる箇所すべてに同じ判断が要る**。本プロジェクトでは「拡張性が要る境界は 1 か 2、集合が閉じている所は 3」を基準に選ぶ。

### 型安全な状態遷移
- **ツール失敗を `Result` の `Err` でループ外に投げず**、`ToolOutput::Error { .. }` という正常値としてモデルへ戻す。
  「失敗してもループは継続」という設計上の不変条件が型に現れる。
- 検証 → 解決 → 実行 → 整形の各段階を `enum` で表し、未検証の呼び出しを実行できないよう型で順序を強制する（typestate）。

```rust
enum ToolOutput {
    Success(ToolResult),
    Error { message: String },   // モデルが回復を試みる
}
```

### Testable（副作用の追い出し）
- `Tool` を trait にすることで、テストでは「固定出力を返すフェイクツール」を登録してループ全体を決定的に検証できる。
- 並行/逐次の差はレイテンシだけで結果は同じになるよう設計し、テストは決定的に。実 I/O（HTTP・プロセス・MCP）はアダプタに閉じ込める。

### 関数型のエッセンス
- ツール実行は「`Vec<ResolvedCall>` → `Vec<ToolOutput>`」の写像。`Concurrent` は `map` の並列版、`Sequential` は逐次 `map`、と捉えると executor の差が明快になる。
- 結果を会話履歴へ戻す処理は「履歴 + 出力群 → 新しい履歴」の純粋な畳み込み。
