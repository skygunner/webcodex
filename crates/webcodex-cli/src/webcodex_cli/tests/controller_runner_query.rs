use super::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

fn body(status: &str) -> Value {
    json!({"success":true,"output":{"runners":[{
        "client_id":"fixture-runner","status":status,"project_inventory":{"sync_state":"complete"}
    }]}})
}

#[test]
fn observation_accepts_only_one_matching_runner_with_a_known_status() {
    for status in ["online", "stale", "offline"] {
        let observed = parse(&body(status), "fixture-runner").unwrap();
        assert_eq!(observed.online, status == "online");
        assert_eq!(observed.project_inventory["sync_state"], "complete");
    }
    assert_eq!(
        parse(&body("unknown"), "fixture-runner").unwrap_err(),
        QueryError::InvalidResponse
    );
    assert_eq!(
        parse(&body("online"), "another-runner").unwrap_err(),
        QueryError::NotVisible
    );
    for invalid in [
        json!({"success":true,"output":{}}),
        json!({"success":false}),
        json!({"success":true,"output":{"runners":[{"client_id":"fixture-runner"}]}}),
    ] {
        assert_eq!(
            parse(&invalid, "fixture-runner").unwrap_err(),
            QueryError::InvalidResponse
        );
    }
    let mut duplicate = body("online");
    let runner = duplicate["output"]["runners"][0].clone();
    duplicate["output"]["runners"]
        .as_array_mut()
        .unwrap()
        .push(runner);
    assert_eq!(
        parse(&duplicate, "fixture-runner").unwrap_err(),
        QueryError::InvalidResponse
    );
}

async fn read_request(stream: tokio::net::TcpStream) -> (BufReader<tokio::net::TcpStream>, Value) {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first).await.unwrap();
    assert_eq!(first.split_whitespace().nth(1), Some("/api/tools/call"));
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await.unwrap();
    (reader, serde_json::from_slice(&bytes).unwrap())
}

async fn exchange(response: Vec<u8>) -> Result<Observation, QueryError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (mut stream, request) = read_request(stream).await;
        assert_eq!(
            request,
            json!({"tool":"list_runners","params":{
                "client_id":"fixture-runner","summary_only":false,"include_projects":false
            }})
        );
        // Early rejection of headers or oversized chunks may close the socket.
        let _ = stream.get_mut().write_all(&response).await;
    });
    let result = query(
        &server,
        "fixture-runner",
        "fixture-secret",
        &ServerHttpOptions {
            no_system_proxy: true,
            ..Default::default()
        },
    )
    .await;
    task.await.unwrap();
    result
}

#[tokio::test]
async fn bounded_query_handles_auth_redirects_and_invalid_responses_without_echoing_bodies() {
    for (status, expected) in [
        (401, QueryError::AccessDenied(401)),
        (403, QueryError::AccessDenied(403)),
        (404, QueryError::ApiUnavailable),
        (405, QueryError::ApiUnavailable),
        (302, QueryError::InvalidResponse),
        (500, QueryError::InvalidResponse),
    ] {
        let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: 14\r\nLocation: http://127.0.0.1:1\r\n\r\nfixture-secret");
        let error = exchange(response.into_bytes()).await.unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("fixture-secret"));
    }
    assert_eq!(
        exchange(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\ninvalid".to_vec())
            .await
            .unwrap_err(),
        QueryError::InvalidResponse
    );
    let body = body("online").to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert!(exchange(response.into_bytes()).await.unwrap().online);
}

#[tokio::test]
async fn response_limit_covers_both_declared_and_chunked_lengths() {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        RESPONSE_LIMIT + 1
    );
    assert_eq!(
        exchange(response.into_bytes()).await.unwrap_err(),
        QueryError::Oversized
    );
    let chunk = "x".repeat(RESPONSE_LIMIT + 1);
    let response = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{chunk}\r\n0\r\n\r\n",
        chunk.len()
    );
    assert_eq!(
        exchange(response.into_bytes()).await.unwrap_err(),
        QueryError::Oversized
    );
}

#[tokio::test]
async fn total_query_timeout_is_bounded_with_a_controlled_clock() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (stream, _) = read_request(stream).await;
        tx.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(stream);
    });
    let query_task = tokio::spawn(async move {
        query(
            &server,
            "fixture-runner",
            "fixture-secret",
            &ServerHttpOptions {
                no_system_proxy: true,
                ..Default::default()
            },
        )
        .await
    });
    rx.await.unwrap();
    tokio::time::pause();
    tokio::time::advance(QUERY_TIMEOUT).await;
    assert_eq!(
        query_task.await.unwrap().unwrap_err(),
        QueryError::Unreachable
    );
    task.abort();
}
