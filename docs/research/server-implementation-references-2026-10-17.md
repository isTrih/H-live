# Server 实施 API 核对（2026-10-17）

本次为 Linux + Docker 服务版实施提供只读参考；仅核对官方固定版本源码、发布与镜像索引，没有启动服务、部署或访问凭据。下面 API 均以 **Salvo v1.0.1** 为准，旧 skill 中 0.94 示例不能代替编译验证。

## 请求体边界与客户端地址

官方 [`request.rs`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/core/src/http/request.rs) 提供：

```rust
pub fn limit_body(&mut self, max_size: usize);
pub fn set_secure_max_size(&mut self, size: usize);
pub fn secure_max_size(&self) -> usize;
pub async fn payload(&mut self) -> ParseResult<&Bytes>;
pub async fn payload_with_max_size(&mut self, max_size: usize) -> ParseResult<&Bytes>;
pub fn remote_addr(&self) -> &SocketAddr;
```

- `limit_body` 设置解析上限，并用 `http_body_util::Limited` 包装底层流；无需信任 Content-Length，可约束 chunked 实际读入量。应在任何解析前调用。
- `payload_with_max_size` 首次读取也用 `Limited::collect`，超限映射 `ParseError::PayloadTooLarge`。已缓存的 payload 会直接返回，因此不能先无限 / 大限读取，再靠较小参数重新验证。`secure_max_size` 无请求专属配置时回落全局默认 64 KiB。
- `remote_addr` 是 **Salvo 自己的枚举**，不是 `std::net::SocketAddr`；[`conn/addr.rs`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/core/src/conn/addr.rs) 提供 `ip(&self) -> Option<std::net::IpAddr>`、`port(&self) -> Option<u16>`、`into_std(self) -> Option<std::net::SocketAddr>`。Unknown / Unix 地址不能当作 IPv4。它代表直接 TCP 对端；代理转发头的信任需由应用明确配置。

## 关闭服务与 feature

[`server.rs`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/core/src/server.rs) 的 `Server::handle(&self) -> ServerHandle` 与 `ServerHandle::stop_graceful(&self, timeout: impl Into<Option<Duration>>)` 可用于信号触发退出；后者返回 `()`，发送停机命令，不是可 await 的停机完成通知。`None` 等待连接自然关闭，`Some(Duration)` 给出连接宽限期；长连接应先主动结束。

[`crates/salvo/Cargo.toml`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/salvo/Cargo.toml) 的默认 feature 包含 `server-handle`；若使用 `default-features = false`，必须显式开启它才能使用 `handle()`。`serve-static`、`cookie`、`test` 也应按实际需求显式选择。HTTP drain 与核心归档完成是不同阶段，服务需要自行等待 `Runtime::shutdown` 和安全销毁。

## TestClient 的 header 与 Cookie

[`RequestBuilder`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/core/src/test/request/builder.rs) 的可用链式调用包括：

```rust
TestClient::post("http://127.0.0.1/api/example")
    .add_header("cookie", "h_live_session=example", true)
    .add_header("x-csrf-token", "example", true)
    .json(&payload)
    .send(&service)
    .await;
```

`add_header(name, value, overwrite)` 第三个参数必填；`true` 替换，`false` 追加。另有 `.bearer_auth(token)`、`.raw_json(string)`、`.body(impl Into<ReqBody>)`。没有持久浏览器 cookie jar 的便捷接口；每个测试请求显式带 Cookie。

启用 `cookie` feature 后，Request 从 Cookie header 填充 cookie jar，`req.cookie(name)` 可读取。测试响应可检查 `res.cookies()`；builder 的 `send` 对 cookie delta 逐项 `insert(SET_COOKIE, ...)`，多个 cookie 的 header 可能只保留最后一项，不能用它证明多 cookie 的真实网络行为。

## 下载与静态页面

[`NamedFile`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/core/src/fs/named_file.rs) 用法：

```rust
NamedFile::builder(server_generated_path)
    .attached_name("report.csv")
    .send(req.headers(), res)
    .await;
```

`NamedFileBuilder::send(self, req_headers: &HeaderMap, res: &mut Response)` 返回 `()`；找不到文件会渲染 404，构建失败也写入响应。路径应来自服务端可信任务记录，鉴权完成后再调用，不能接受用户任意文件路径。[官方文件发送说明](https://salvo.rs/guide/topics/send-file)

[`StaticDir`](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/serve-static/src/dir.rs) 支持 `StaticDir::new(path).defaults("index.html").auto_list(false).include_dot_files(false).fallback("index.html")`；官方 [`serve-static` 测试](https://github.com/salvo-rs/salvo/blob/v1.0.1/crates/serve-static/src/lib.rs) 使用 `Router::with_path("{*path}")`，并覆盖路径穿越与符号链接越界。SPA fallback 只应位于网页静态分支，避免未知 API 路径返回 HTML。只挂载构建后的公开 UI 目录；归档、配置与密钥不进入该目录。

## CI 与 Docker 的已核实版本

- **actions/checkout v7.0.1**：官方发布页显示稳定发布；GitHub tag API 将它解析为 commit **`3d3c42e5aac5ba805825da76410c181273ba90b1`**。可写 `uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`。[发布](https://github.com/actions/checkout/releases/tag/v7.0.1)、[tag API](https://api.github.com/repos/actions/checkout/git/ref/tags/v7.0.1)
- 官方 [`action.yml`](https://github.com/actions/checkout/blob/3d3c42e5aac5ba805825da76410c181273ba90b1/action.yml) 使用 Node 24；官方 [README](https://github.com/actions/checkout#checkout-v5) 说明 Node 24 要求 runner 至少 2.327.1。当前受维护的 GitHub 托管 runner 是合适目标；自托管 runner 须自行核对版本。仓库读取工作流用 `permissions: contents: read`、`persist-credentials: false` 即可。
- 官方 [`library/rust` 索引](https://github.com/docker-library/official-images/blob/master/library/rust) 本次列出 **`rust:1.99.1-bookworm`** 与 **`rust:1.99.1-slim-bookworm`**，对应 Docker Rust 仓库提交 `1b9592f08592a09d61dbc6d27a5c7cf2be1c2e92`。Rust 1.99.1 高于 Salvo v1.0.1 声明的最低 1.94。[固定 Dockerfile](https://github.com/rust-lang/docker-rust/blob/1b9592f08592a09d61dbc6d27a5c7cf2be1c2e92/stable/bookworm/Dockerfile)、[Salvo MSRV](https://github.com/salvo-rs/salvo/blob/v1.0.1/Cargo.toml)

以上仅证明上游 API、发布引用和官方镜像标签存在；项目本身仍须完成 Cargo 编译、HTTP 行为与 Linux 容器构建验证。未由本次调研拉取或启动镜像。
