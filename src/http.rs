//! GPUI's image HTTP transport. Client/TLS setup and downloads happen only
//! when a visible image requests them, on smol's blocking pool.
use std::{sync::OnceLock, time::Duration};

use futures::future::BoxFuture;
use gpui::http_client::{
    AsyncBody, HttpClient, RedirectPolicy, Request, Response, Result, Url, http::HeaderValue,
};

pub struct ImageHttpClient;

impl HttpClient for ImageHttpClient {
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(&self, request: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        Box::pin(async move {
            smol::unblock(move || {
                // Two lazy clients preserve GPUI's per-request redirect policy.
                static FOLLOW: OnceLock<reqwest::blocking::Client> = OnceLock::new();
                static DIRECT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
                let follow = !matches!(
                    request.extensions().get::<RedirectPolicy>(),
                    None | Some(RedirectPolicy::NoFollow)
                );
                let client = if follow { &FOLLOW } else { &DIRECT }.get_or_init(|| {
                    reqwest::blocking::Client::builder()
                        .user_agent("mdview")
                        .timeout(Duration::from_secs(30))
                        .redirect(if follow {
                            reqwest::redirect::Policy::limited(10)
                        } else {
                            reqwest::redirect::Policy::none()
                        })
                        .build()
                        .expect("HTTP client initialization failed")
                });
                let response = client
                    .request(request.method().clone(), request.uri().to_string())
                    .headers(request.headers().clone())
                    .send()?;
                let mut result = Response::builder().status(response.status());
                *result.headers_mut().unwrap() = response.headers().clone();
                Ok(result.body(response.bytes()?.to_vec().into())?)
            })
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::AsyncReadExt;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    #[test]
    fn downloads_images_follows_redirects_and_preserves_errors() {
        assert_eq!(ImageHttpClient.type_name(), "mdview::http::ImageHttpClient");
        assert!(ImageHttpClient.user_agent().is_none());
        assert!(ImageHttpClient.proxy().is_none());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for reply in [
                "HTTP/1.1 302 Found\r\nLocation: /image.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage",
                "HTTP/1.1 404 Not Found\r\nContent-Length: 7\r\nConnection: close\r\n\r\nmissing",
                "HTTP/1.1 302 Found\r\nLocation: /image.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(&stream);
                loop {
                    let mut line = String::new();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        smol::block_on(async {
            let mut response = ImageHttpClient.get(&url, ().into(), true).await.unwrap();
            assert_eq!(response.status(), 200);
            let mut body = String::new();
            response.body_mut().read_to_string(&mut body).await.unwrap();
            assert_eq!(body, "image");
            let mut response = ImageHttpClient.get(&url, ().into(), true).await.unwrap();
            assert_eq!(response.status(), 404);
            body.clear();
            response.body_mut().read_to_string(&mut body).await.unwrap();
            assert_eq!(body, "missing");
            let response = ImageHttpClient.get(&url, ().into(), false).await.unwrap();
            assert_eq!(response.status(), 302);
        });
        server.join().unwrap();
    }
}
