mod echo_model;

use std::io::{self, BufRead, Write};

use agent_core::{
    message::{AssistantContent, UserMessage},
    ports::EmptyToolRegistry,
    ConversationHistory, TurnRunner,
};
use echo_model::EchoLanguageModel;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runner = TurnRunner::new(EchoLanguageModel, EmptyToolRegistry);
    let mut history = ConversationHistory::new();

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = io::stdout();

    println!("agent-cli (echo mode)");
    println!("Type \"exit\" to quit.");
    println!();

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
