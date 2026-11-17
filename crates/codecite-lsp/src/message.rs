//! The JSON-RPC 2.0 messages codecite sends and receives, written by hand
//! over `serde_json::Value` rather than taken from `lsp-types` (#233).

use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// JSON-RPC's `InvalidParams`.
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC's `MethodNotFound`.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// LSP's `RequestCancelled`.
pub const REQUEST_CANCELLED: i64 = -32800;
/// LSP's `ContentModified`.
pub const CONTENT_MODIFIED: i64 = -32801;

/// A request id: JSON-RPC allows a number or a string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Id {
    /// A numeric id; codecite allocates these.
    Number(i64),
    /// A string id; servers may use these for their own requests.
    String(String),
}

impl Id {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(n) => n.as_i64().map(Id::Number),
            Value::String(s) => Some(Id::String(s.clone())),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Id::Number(n) => json!(n),
            Id::String(s) => json!(s),
        }
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseError {
    /// The error code.
    pub code: i64,
    /// A short description.
    pub message: String,
    /// Extra detail, if any.
    pub data: Option<Value>,
}

/// One decoded JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// A request, which expects a response with the same id.
    Request {
        /// Its id.
        id: Id,
        /// Its method.
        method: String,
        /// Its params; `Null` when absent.
        params: Value,
    },
    /// A notification, which gets no response.
    Notification {
        /// Its method.
        method: String,
        /// Its params; `Null` when absent.
        params: Value,
    },
    /// A response to a request. `id` is `None` when the server could not
    /// read the request's id.
    Response {
        /// The id of the request it answers.
        id: Option<Id>,
        /// The result, or the error.
        outcome: std::result::Result<Value, ResponseError>,
    },
}

impl Message {
    /// Classifies a decoded frame body.
    pub fn from_value(value: Value) -> Result<Self> {
        let invalid = |reason: &str| Error::InvalidMessage {
            reason: reason.to_owned(),
        };
        let Value::Object(mut object) = value else {
            return Err(invalid("not an object"));
        };
        let id = object.remove("id");
        if let Some(method) = object.get("method") {
            let method = method
                .as_str()
                .ok_or_else(|| invalid("method is not a string"))?
                .to_owned();
            let params = object.remove("params").unwrap_or(Value::Null);
            return Ok(match id {
                None => Message::Notification { method, params },
                Some(id) => Message::Request {
                    id: Id::from_value(&id).ok_or_else(|| invalid("bad request id"))?,
                    method,
                    params,
                },
            });
        }
        let id = match id {
            None | Some(Value::Null) => None,
            Some(id) => Some(Id::from_value(&id).ok_or_else(|| invalid("bad response id"))?),
        };
        // A null `error` is forbidden but sent by some servers: it means none.
        let error = object.remove("error").filter(|e| !e.is_null());
        let outcome = if let Some(error) = error {
            Err(response_error(error).ok_or_else(|| invalid("bad error object"))?)
        } else if let Some(result) = object.remove("result") {
            Ok(result)
        } else {
            return Err(invalid("neither a request, a notification nor a response"));
        };
        Ok(Message::Response { id, outcome })
    }

    /// Encodes the message as a JSON-RPC 2.0 object.
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("jsonrpc".into(), json!("2.0"));
        match self {
            Message::Request { id, method, params } => {
                object.insert("id".into(), id.to_value());
                object.insert("method".into(), json!(method));
                if !params.is_null() {
                    object.insert("params".into(), params.clone());
                }
            }
            Message::Notification { method, params } => {
                object.insert("method".into(), json!(method));
                if !params.is_null() {
                    object.insert("params".into(), params.clone());
                }
            }
            Message::Response { id, outcome } => {
                object.insert("id".into(), id.as_ref().map_or(Value::Null, Id::to_value));
                match outcome {
                    Ok(result) => object.insert("result".into(), result.clone()),
                    Err(error) => {
                        let mut e = json!({"code": error.code, "message": error.message});
                        if let Some(data) = &error.data {
                            e["data"] = data.clone();
                        }
                        object.insert("error".into(), e)
                    }
                };
            }
        }
        Value::Object(object)
    }
}

fn response_error(value: Value) -> Option<ResponseError> {
    Some(ResponseError {
        code: value.get("code")?.as_i64()?,
        message: value.get("message")?.as_str()?.to_owned(),
        data: value.get("data").cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_requests_notifications_and_responses() {
        let request = json!({"jsonrpc": "2.0", "id": "a", "method": "m", "params": [1]});
        assert_eq!(
            Message::from_value(request).unwrap(),
            Message::Request {
                id: Id::String("a".into()),
                method: "m".into(),
                params: json!([1])
            }
        );
        let note = json!({"jsonrpc": "2.0", "method": "n"});
        assert_eq!(
            Message::from_value(note).unwrap(),
            Message::Notification {
                method: "n".into(),
                params: Value::Null
            }
        );
        let error = json!({"jsonrpc": "2.0", "id": 3,
            "error": {"code": -32601, "message": "nope", "data": {"x": 1}}});
        assert_eq!(
            Message::from_value(error).unwrap(),
            Message::Response {
                id: Some(Id::Number(3)),
                outcome: Err(ResponseError {
                    code: METHOD_NOT_FOUND,
                    message: "nope".into(),
                    data: Some(json!({"x": 1}))
                })
            }
        );
        let null_result = json!({"jsonrpc": "2.0", "id": 4, "result": null});
        assert_eq!(
            Message::from_value(null_result).unwrap(),
            Message::Response {
                id: Some(Id::Number(4)),
                outcome: Ok(Value::Null)
            }
        );
    }

    #[test]
    fn a_null_error_beside_a_result_is_a_success() {
        // JSON-RPC forbids it, but some servers send it.
        let value = json!({"jsonrpc": "2.0", "id": 5, "result": [1], "error": null});
        assert_eq!(
            Message::from_value(value).unwrap(),
            Message::Response {
                id: Some(Id::Number(5)),
                outcome: Ok(json!([1]))
            }
        );
    }

    #[test]
    fn refuses_what_is_not_json_rpc() {
        for value in [json!([1]), json!({"id": 1}), json!({"method": 7})] {
            assert!(matches!(
                Message::from_value(value),
                Err(Error::InvalidMessage { .. })
            ));
        }
    }

    #[test]
    fn encodes_and_decodes_unchanged() {
        let messages = [
            Message::Request {
                id: Id::Number(1),
                method: "initialize".into(),
                params: json!({"rootUri": "file:///w"}),
            },
            Message::Notification {
                method: "exit".into(),
                params: Value::Null,
            },
            Message::Response {
                id: Some(Id::String("x".into())),
                outcome: Ok(json!(null)),
            },
            Message::Response {
                id: None,
                outcome: Err(ResponseError {
                    code: -32700,
                    message: "parse error".into(),
                    data: None,
                }),
            },
        ];
        for message in messages {
            let value = message.to_value();
            assert_eq!(value["jsonrpc"], "2.0");
            assert_eq!(Message::from_value(value).unwrap(), message);
        }
    }
}
