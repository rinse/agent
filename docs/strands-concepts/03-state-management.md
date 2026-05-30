# 03. State Management

> 出典: <https://strandsagents.com/docs/user-guide/concepts/agents/state/>

## 3 種類の状態

エージェントの状態は性質の異なる 3 つに分かれる。**「モデルの文脈に載るか否か」「生存期間」**が分類軸。

| 状態 | 内容 | モデル文脈に載る？ | 生存期間 |
| --- | --- | --- | --- |
| **Conversation History**（会話履歴） | 全ユーザー/アシスタントメッセージ、ツール呼び出しと結果 | **載る**（推論に渡る） | 会話全体。既存メッセージで初期化して継続も可 |
| **Agent State**（アプリ状態） | 会話文脈の外に置くキーバリュー情報。ツールから読めるが推論には渡さない | 載らない | エージェント横断 |
| **Invocation State**（呼び出し状態） | 1 回の呼び出し内で持つ一時的文脈データ。既定 `{}` | 載らない | 単一呼び出し内。Hooks/ツール間で共有 |

## 構造と管理

- フレームワークは Agent State に **JSON シリアライズ可能**であることを要求する（永続化のため）。
- 既定の conversation manager はスライディングウィンドウで、必要に応じ古いメッセージを自動削除する。
- ツールは `ToolContext` 経由で状態にアクセスし、**直接 read/mutate** できる。変更は以降のツール呼び出しに持続する。
- 直接ツール呼び出しは `record_direct_tool_call` で会話履歴に記録するか選べる。

## 永続化

- **Session Management**: セッションをまたいだ自動永続化。
- **Snapshots**: 任意時点の手動キャプチャ／復元。

---

## 本プロジェクト（Rust）への設計示唆

### 型安全 / 関心の分離
- 「モデルに渡る状態」と「渡らない状態」を**別の型**にして混同を防ぐ。これは文脈汚染（不要情報がプロンプトに漏れる）を型で防ぐということ。

```rust
/// モデル推論に渡る（= 会話の作業記憶）
struct ConversationHistory { messages: Vec<Message> }

/// 推論には渡らない、ツールが読み書きするアプリ状態。永続化のため Serialize 必須
#[derive(Serialize, Deserialize, Default)]
struct AgentState(HashMap<String, serde_json::Value>);

/// 1 回の呼び出しに閉じた一時状態。Hooks/ツール間で共有
#[derive(Default)]
struct InvocationState(HashMap<String, Box<dyn Any + Send>>);
```

### SOLID
- 永続化は `trait SessionStore { fn save(..); fn load(..); }` として分離（DIP）。インメモリ実装でテスト、ファイル/DB 実装で本番。
- ツールが状態へ触る経路は `ToolContext` のような**明示的な受け渡し**にして、グローバル可変状態を避ける。

### Testable（副作用の追い出し）
- `AgentState` を `Serialize`/`Deserialize` に縛ると、スナップショットの round-trip（save→load で一致）をテストしやすい。
- 永続化を trait 越しにすれば、ストレージ I/O を伴わずに状態遷移ロジックを検証できる。

### 関数型のエッセンス
- 会話履歴は不変に積み上げ、可変が必要なアプリ状態は明示的に「現在状態 → 次状態」で受け渡す。可変領域を最小化し、どこで状態が変わるかを局所化する。
