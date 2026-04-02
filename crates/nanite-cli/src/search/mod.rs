mod assets;
mod server;
mod service;

use crate::cli::{SearchArgs, SearchCommands, SearchIndexCommands};
use crate::context::ContextState;
use anyhow::{Result, bail};
use service::{RefreshMode, parse_search_request};

pub fn command_search(context: &ContextState, args: SearchArgs) -> Result<()> {
    let SearchArgs {
        command,
        query,
        repo,
        path,
        file,
        lang,
        limit,
        json,
    } = args;

    match command {
        Some(SearchCommands::Serve { host, port }) => server::serve(context, &host, port),
        Some(SearchCommands::Index {
            command: SearchIndexCommands::Rebuild,
        }) => {
            let open = service::SearchEngine::open(context, RefreshMode::ForceRebuild)?;
            let report = open.report;
            println!(
                "rebuilt search index at {} ({} files, {} lines)",
                report.index_path, report.files_indexed, report.lines_indexed
            );
            Ok(())
        }
        None => {
            if query.as_deref().is_none_or(str::is_empty) {
                bail!(
                    "search requires a query; use 'nanite search <query>' or 'nanite search serve'"
                );
            }
            let request = parse_search_request(
                query.as_deref().unwrap_or_default(),
                repo,
                path,
                file,
                lang,
                limit,
            );
            let open = service::SearchEngine::open(context, RefreshMode::RefreshIfNeeded)?;
            let response = open.engine.search(&request)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                for hit in response.hits {
                    println!("{}:{}:{}", hit.path, hit.line_number, hit.text);
                }
            }
            Ok(())
        }
    }
}
