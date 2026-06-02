use std::io::{self, BufRead, Write};

use adapters::openai::OpenAiModel;
use agent_core::{
    message::{AssistantContent, UserMessage},
    ports::EmptyToolRegistry,
    ConversationHistory, TurnRunner,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "http://localhost:1234/v1".into());
    let model_name = std::env::var("OPENAI_MODEL").unwrap_or_default();

    let mut model = OpenAiModel::new(&base_url, &model_name);

    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        model = model.with_api_key(key);
    }
    if let Ok(prompt) = std::env::var("SYSTEM_PROMPT") {
        model = model.with_system_prompt(prompt);
    }

    let runner = TurnRunner::new(model, EmptyToolRegistry);
    let mut history = ConversationHistory::new();

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = io::stdout();

    eprintln!("agent-cli (model: {model_name})");
    eprintln!("  base_url: {base_url}");
    eprintln!("Type \"exit\" to quit.");
    eprintln!();

    loop {
        print!("you> ");
        stdout.flush()?;

        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let input = line.trim_end();
        if input.is_empty() {
            continue;
        }
        if input == "exit" || input == "quit" {
            break;
        }

        history.push(UserMessage::text(input));

        match runner.run(&mut history).await {
            Ok(response) => {
                for content in &response.content {
                    match content {
                        AssistantContent::Text(text) => println!("agent> {text}"),
                        AssistantContent::ToolUse(call) => {
                            println!("agent> [tool_use: {}]", call.name);
                        }
                    }
                }
            }
            Err(e) => eprintln!("error: {e}"),
        }
    }

    Ok(())
}
