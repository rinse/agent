use std::io::{self, BufRead, Write};

use adapters::EchoLanguageModel;
use agent_core::{
    message::{AssistantContent, UserMessage},
    ports::EmptyToolRegistry,
    ConversationHistory, TurnRunner,
};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let runner = TurnRunner::new(EchoLanguageModel, EmptyToolRegistry);
    let mut history = ConversationHistory::new();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    println!("agent-cli (echo mode)");
    println!("Type \"exit\" to quit.");
    println!();

    loop {
        print!("you> ");
        stdout.flush().unwrap();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap() == 0 {
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
}
