use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Stdout};

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Persistent stdio transport
// ---------------------------------------------------------------------------

/// Reads JSON-RPC message lines from stdin.
///
/// The reader half is kept separate from the writer so the read loop can keep
/// accepting messages while spawned handlers write their responses. BufReader
/// MUST be created once and reused — recreating it drops buffered data.
pub struct StdioReader {
    reader: BufReader<tokio::io::Stdin>,
    byte_buf: Vec<u8>,
}

impl Default for StdioReader {
    fn default() -> Self {
        Self::new()
    }
}

impl StdioReader {
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(tokio::io::stdin()),
            byte_buf: Vec::new(),
        }
    }

    /// Maximum allowed message length (10 MB). The budget is applied to the
    /// reader itself, so a client that never sends a newline is cut off at the
    /// limit rather than being allowed to grow the buffer without bound first.
    const MAX_LINE_LEN: usize = 10_000_000;

    /// Read one raw JSON-RPC message line from stdin. Returns `None` on EOF.
    /// Blank lines are silently skipped so that stray newlines do not kill the
    /// MCP connection.
    ///
    /// Parsing is left to the caller so that malformed JSON can be answered
    /// with a JSON-RPC parse error instead of being indistinguishable from a
    /// transport failure.
    pub async fn read_message(&mut self) -> io::Result<Option<String>> {
        loop {
            self.byte_buf.clear();

            // Bound the read itself: `take` stops after the budget, so an
            // endless newline-less stream cannot balloon the buffer.
            let mut limited = (&mut self.reader).take(Self::MAX_LINE_LEN as u64 + 1);
            let n = limited.read_until(b'\n', &mut self.byte_buf).await?;

            if n == 0 {
                return Ok(None); // true EOF
            }

            // No terminating newline within the budget means the message is
            // over-long. The stream cannot be resynchronised safely, so this is
            // fatal rather than a skippable message.
            if n > Self::MAX_LINE_LEN && !self.byte_buf.ends_with(b"\n") {
                self.byte_buf.clear();
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSON-RPC message exceeds 10 MB limit",
                ));
            }

            let line = String::from_utf8_lossy(&self.byte_buf);
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // skip blank lines, don't treat as EOF
            }
            return Ok(Some(trimmed.to_string()));
        }
    }
}

/// Writes JSON-RPC responses to stdout.
///
/// Handlers run concurrently, so this is shared behind a mutex. Each
/// `write_response` emits one complete newline-terminated line while the caller
/// holds the lock, which is what keeps concurrent responses from interleaving
/// mid-line.
pub struct StdioWriter {
    writer: Stdout,
}

impl Default for StdioWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl StdioWriter {
    pub fn new() -> Self {
        Self {
            writer: tokio::io::stdout(),
        }
    }

    /// Serialize a response to compact JSON, write as one line to stdout, flush.
    /// Uses compact serialization — MCP messages MUST NOT contain embedded newlines.
    pub async fn write_response(&mut self, resp: &JsonRpcResponse) -> io::Result<()> {
        let json = serde_json::to_string(resp).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize response: {e}"),
            )
        })?;

        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;

        Ok(())
    }
}
