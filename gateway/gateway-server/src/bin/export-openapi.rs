//! 输出 OpenAPI JSON 到 stdout。供 `npm run gen:openapi` 调用。
//!
//! 不启动服务器，不依赖任何运行时状态，直接从 OpenAPI 宏生成。

use gateway_server::api::ApiDoc;
use utoipa::OpenApi;

fn main() {
    let doc = ApiDoc::openapi();
    let json = serde_json::to_string_pretty(&doc).expect("serialize OpenAPI failed");
    println!("{json}");
}
