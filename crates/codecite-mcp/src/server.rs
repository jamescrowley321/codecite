//! The MCP server: one tool per operation, each returning the JSON the
//! matching CLI command prints.

use std::path::PathBuf;

use codecite_core::Corpora;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Json;
use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router};

use crate::Result;

/// codecite's MCP server. It answers from the corpus files in one data
/// directory and holds no state between calls.
#[derive(Debug, Clone)]
pub struct Server {
    /// Where the corpus files are: `<data dir>/<corpus>.db`.
    data_dir: PathBuf,
    /// The tools, built from the `#[tool]` methods below.
    tool_router: ToolRouter<Self>,
}

impl Server {
    /// A server answering from the corpora in `data_dir`.
    pub fn new(data_dir: PathBuf) -> Self {
        Server {
            data_dir,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl Server {
    /// The `corpora` tool: the same JSON as `codecite corpora`.
    #[tool(
        description = "List the corpora this server can search: each one's age, size, source count and whether it is stale. A stale corpus is refused unless a query allows it."
    )]
    async fn corpora(&self) -> Result<Json<Corpora>, String> {
        // SQLite blocks. Running it on tokio's blocking pool keeps this
        // thread free to read and answer other messages meanwhile.
        let dir = self.data_dir.clone();
        match tokio::task::spawn_blocking(move || codecite_core::corpora(&dir)).await {
            Ok(Ok(corpora)) => Ok(Json(corpora)),
            Ok(Err(err)) => Err(with_causes(&err)),
            Err(join) => Err(format!("listing corpora failed: {join}")),
        }
    }
}

/// `err`'s message followed by each of its causes'. An agent sees only
/// this text, so it must say what went wrong and not only where.
fn with_causes(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut cause = err.source();
    while let Some(err) = cause {
        text.push_str(": ");
        text.push_str(&err.to_string());
        cause = err.source();
    }
    text
}

#[tool_handler(
    router = self.tool_router,
    name = "codecite",
    instructions = "codecite answers from indexed code and specs, with a citation on every result. Start with `corpora` to see what is indexed and how fresh it is."
)]
impl ServerHandler for Server {}

/// Runs the server on stdin and stdout until the client closes stdin.
///
/// stdout carries only MCP messages, so anything else codecite prints goes
/// to stderr.
pub fn serve_stdio(data_dir: PathBuf) -> Result<()> {
    // One thread is plenty for one client, and the blocking pool still
    // runs the SQLite work beside it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let running = Server::new(data_dir)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(Box::new)?;
        running.waiting().await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codecite_core::{SourceRecord, Store};
    use rmcp::ClientHandler;
    use rmcp::model::{CallToolRequestParams, ClientConfig};

    /// A client that only calls tools.
    #[derive(Default)]
    struct Client;

    impl ClientHandler for Client {
        fn get_info(&self) -> ClientConfig {
            ClientConfig::default()
        }
    }

    /// A data directory holding one corpus, `demo`, with one source.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
        store
            .upsert_source(&SourceRecord {
                name: "app",
                kind: "local",
                url: "/src/app",
                revision: "0123456789abcdef0123456789abcdef01234567",
                license_spdx: Some("MIT"),
                license_class: "adapt",
                is_current: true,
                obsoleted_by: None,
                updated_by: None,
            })
            .unwrap();
        store.set_indexed_at(30).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_client_lists_the_tools_and_calls_corpora() {
        let dir = fixture();
        // An in-memory pipe stands in for stdio: the server reads one end,
        // the client the other.
        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(Server::new(dir.path().to_path_buf()).serve(server_end));
        let client = Client.serve(client_end).await.unwrap();
        let server = server.await.unwrap().unwrap();

        let info = client.peer_info().unwrap();
        assert_eq!(info.server_info.as_ref().unwrap().name, "codecite");

        let tools = client.list_all_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(names, ["corpora"]);
        let corpora = &tools[0];
        assert_eq!(corpora.input_schema.get("type").unwrap(), "object");
        // The output schema comes from `Corpora`'s Rust type.
        let output = serde_json::Value::Object((*corpora.output_schema.clone().unwrap()).clone());
        assert_eq!(output["properties"]["corpora"]["type"], "array");

        let result = client
            .call_tool(CallToolRequestParams::new("corpora"))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["corpora"][0]["corpus"], "demo");
        assert_eq!(structured["corpora"][0]["sources"], 1);
        assert_eq!(structured["corpora"][0]["stale"], false);
        // The text is the same JSON, for clients that read only text.
        let text = &result.content[0].as_text().unwrap().text;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text).unwrap(),
            structured
        );
        assert_eq!(
            structured,
            serde_json::to_value(codecite_core::corpora(dir.path()).unwrap()).unwrap()
        );

        client.cancel().await.unwrap();
        server.waiting().await.unwrap();
    }

    #[tokio::test]
    async fn an_unreadable_data_directory_is_a_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the directory should be.
        let file = dir.path().join("data");
        std::fs::write(&file, "x").unwrap();
        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(Server::new(file).serve(server_end));
        let client = Client.serve(client_end).await.unwrap();
        let server = server.await.unwrap().unwrap();

        let result = client
            .call_tool(CallToolRequestParams::new("corpora"))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        let text = &result.content[0].as_text().unwrap().text;
        assert!(text.starts_with("I/O error: "), "{text}");
        assert!(text.len() > "I/O error: ".len(), "{text}");

        client.cancel().await.unwrap();
        server.waiting().await.unwrap();
    }
}
