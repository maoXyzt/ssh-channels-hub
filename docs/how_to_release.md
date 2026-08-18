# SSH Channels Hub Release Guide / 发版手册

> English | [中文](#中文)

## English

This guide is for repository maintainers. For installation and usage, see the
[README](../README.md). For development details, see
[Architecture](./architecture.md) and [Module design](./modules.md).

### 1. Overview

Releases have two stages: local `cargo release` updates the version, creates the
commit and tag, and pushes them; CI reacts to the `v*` tag, verifies and builds
the project, creates the GitHub Release, and publishes to crates.io and PyPI.

```text
Local: cargo release patch --execute
  |- update Cargo.toml and Cargo.lock
  |- commit "release: 0.2.1"
  |- tag v0.2.1
  `- push branch and tag
             |
GitHub Actions build.yml
  |- preflight
  |- verify-tag
  |- lint (fmt, clippy, tests)
  |- changelog (git-cliff)
  |- build (Linux, macOS, Windows)
  |- wheels (Linux, macOS, Windows)
  |- release (archives and checksums)
  |- publish (crates.io)
  `- publish-pypi (OIDC)
```

`release`, `publish`, and `publish-pypi` depend on successful lint and tag
verification. Maturin reads the PyPI version from `Cargo.toml`, so
`cargo release` remains the single version source.

### 2. One-time setup

#### 2.1 Install `cargo-release`

```bash
cargo install cargo-release --version 1.1.2 --locked
```

Update the pinned version only after verifying its dry-run against this workflow.

#### 2.2 Configure the crates.io token

1. Create a token at <https://crates.io/settings/tokens> with `publish-update`
   permission, plus `publish-new` for the first release, restricted to
   `ssh-channels-hub`.
2. Add it to the GitHub repository Actions secrets as
   `CARGO_REGISTRY_TOKEN`.

This is required only for initial setup and token rotation.

#### 2.3 Configure the PyPI Trusted Publisher

PyPI uses OIDC instead of a long-lived token:

1. Create a GitHub environment named `pypi`.
2. At <https://pypi.org/manage/account/publishing/>, add a pending publisher:
   - Project: `ssh-channels-hub`
   - Owner: `maoXyzt`
   - Repository: `ssh-channels-hub`
   - Workflow: `build.yml`
   - Environment: `pypi`
3. The first successful release converts it to a regular publisher.

If this setup is missing, only `publish-pypi` fails. Correct it and rerun that
job without creating another tag.

### 3. Pre-release checks

#### 3.1 Use conventional commits

`git-cliff` builds release notes from commits since the previous tag:

| Prefix | Group | Included |
|---|---|---|
| `feat:` | Features | yes |
| `fix:` | Bug Fixes | yes |
| `perf:` | Performance | yes |
| `refactor:` | Refactor | yes |
| `docs:` | Documentation | yes |
| `chore:` | Miscellaneous | no |
| `ci:` | CI/CD | no |
| Other | - | no |

See [cliff.toml](../cliff.toml). Fix incorrect commit prefixes before release
with amend or rebase; otherwise they will not appear in release notes.

#### 3.2 Run lint and tests locally

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

#### 3.3 Optional publish dry run

```bash
cargo publish --dry-run --locked --registry crates-io
```

CI runs the same metadata check on normal pushes and pull requests.

### 4. Release procedure

#### 4.1 Switch to the release branch

`cargo-release` allows `main` or `master` by default. Merge `dev` first:

```bash
git checkout main
git pull
git merge --ff-only dev   # or merge a pull request into main
```

To release directly from `dev`, add it to `allow-branch` under
`[package.metadata.release]` in `Cargo.toml`.

#### 4.2 Preview the release

Without `--execute`, these commands are dry runs:

```bash
cargo release patch          # 0.2.0 -> 0.2.1
cargo release minor          # 0.2.0 -> 0.3.0
cargo release 1.0.0          # exact version
```

Confirm that the preview reports it would update both Cargo files, create
`release: <new-version>`, tag `v<new-version>`, and push to `origin`.

#### 4.3 Execute

```bash
cargo release patch --execute
```

This updates `Cargo.toml` and `Cargo.lock`, commits, creates an annotated tag,
and pushes the branch and tags.

#### 4.4 Monitor CI

Watch the [build workflow](https://github.com/maoXyzt/ssh-channels-hub/actions).
After all jobs pass, confirm the GitHub Release archives and checksums, the new
[crates.io version](https://crates.io/crates/ssh-channels-hub), and the new
[PyPI version](https://pypi.org/project/ssh-channels-hub/).

### 5. Release safeguards

| Risk | Safeguard |
|---|---|
| Tag and manifest versions differ | `cargo release` creates both; `verify-tag` checks them again |
| Unexpected publish during `cargo release` | `[package.metadata.release].publish = false` disables its local publish step; `[package].publish` remains unset so CI can run `cargo publish` |
| One platform fails after publication | Publish jobs depend on all builds or wheels |
| Formatting, clippy, or tests fail | Release and publish jobs depend on `lint` |
| Package metadata regresses | Normal pushes run `publish-dry-run` |
| Branch and tag trigger duplicate builds | `preflight` skips the duplicate release commit run |
| PyPI token leaks | PyPI uses OIDC Trusted Publisher |

### 6. Troubleshooting

#### `cargo release` reports `branch 'dev' is not whitelisted`

Switch to `main`, or add `dev` to `allow-branch` as described in section 4.1.

#### `publish` reports `crate version is already uploaded`

crates.io versions are immutable. Bump to a new version. Remove the incomplete
tag if necessary before retrying.

#### `verify-tag` fails

The tag does not match `Cargo.toml`, usually because it was created manually:

```bash
git tag -d v<wrong>
git push --delete origin v<wrong>
cargo release <correct> --execute
```

#### Release notes are empty or omit a commit

Check `git log --oneline v<prev>..HEAD` against the prefixes in section 3.1.
Correct history before release; after release, document the omission in the next
version rather than rewriting published history.

#### `lint` fails on a clippy rule

Fix the code or add a justified local lint configuration. Do not weaken
`-D warnings` to bypass the release gate.

#### `publish-pypi` reports `invalid-publisher` or 401

Verify the PyPI project, owner, repository, workflow, and environment values,
then rerun only `publish-pypi`.

#### `publish-pypi` reports `File already exists`

PyPI files are immutable. Bump the version and publish again.

### 7. Withdrawing a release

crates.io versions cannot be deleted, but they can be yanked:

```bash
cargo yank --version 0.2.1
cargo yank --version 0.2.1 --undo
```

Yank the PyPI version from its project page. A GitHub Release can be deleted,
and its tag can be removed with `git push --delete origin v0.2.1`.

## 中文

本文档仅面向仓库管理员。

仅安装与使用请看仓库根目录的 [README.md](../README.md)；开发相关参见 [架构文档](./architecture.md)、[模块文档](./modules.md)。

### 1. 总览

发版分两段:**本地** 用 `cargo release` 同时 bump 版本号 + 打 tag + 推送；**CI** 监听 `v*` tag，自动跑 lint/test、编译三个平台二进制和 Python wheel、生成 changelog、创建 GitHub Release，并发布到 crates.io 和 PyPI。

```
本地: cargo release patch --execute
        │   bump Cargo.toml + Cargo.lock
        │   commit "release: 0.2.1"
        │   tag v0.2.1
        └─→ git push (branch + tag)
                            │
GitHub Actions (build.yml) ←┘
  ├─ preflight        (识别 release: 提交,避免双跑)
  ├─ verify-tag       (tag ↔ Cargo.toml version 校验)
  ├─ lint             (fmt + clippy -D warnings + cargo test)
  ├─ changelog        (git-cliff 从 conventional commits 生成 release notes)
  ├─ build (×3)       (linux-gnu / aarch64-darwin / windows-msvc)
  ├─ wheels (×3)      (同三平台,maturin 打 Python wheel)
  ├─ release          (GitHub Release + tarball/zip + .sha256)
  ├─ publish          (cargo publish → crates.io)
  └─ publish-pypi     (OIDC → PyPI)
```

`release`、`publish` 和 `publish-pypi` 都强依赖 `lint` 和 `verify-tag`，任何一项失败都不会发布。PyPI 版本由 maturin 直接读取 `Cargo.toml`,`cargo release` 仍是唯一版本来源。

### 2. 一次性准备

#### 2.1 安装 cargo-release

```bash
cargo install cargo-release --version 1.1.2 --locked
```

只有用 dry-run 验证新版本与本流程一致后，才更新固定版本。

#### 2.2 准备 crates.io token(仓库管理员一次性)

1. 在 <https://crates.io/settings/tokens> 创建 token，scope 勾选 `publish-update`(首次发布额外勾 `publish-new`)，crate 限定为 `ssh-channels-hub`。
2. GitHub 仓库 → **Settings → Secrets and variables → Actions → New repository secret**:
   - Name: `CARGO_REGISTRY_TOKEN`
   - Value: 上一步生成的 token

token 只在第一次配置和需要轮换时操作，日常发版无需关心。

#### 2.3 准备 PyPI Trusted Publisher(仓库管理员一次性)

PyPI 使用 Trusted Publisher(OIDC)，不需要长期 token。首次发布前:

1. 在 GitHub 仓库的 **Settings → Environments** 创建名为 `pypi` 的 environment。
2. 登录 <https://pypi.org/manage/account/publishing/>，添加 pending publisher:
   - PyPI Project Name: `ssh-channels-hub`
   - Owner: `maoXyzt`
   - Repository name: `ssh-channels-hub`
   - Workflow filename: `build.yml`
   - Environment name: `pypi`
3. 第一次发布成功后，pending publisher 会自动转为正式 publisher。

没配置或字段不一致时，只有 `publish-pypi` job 会失败；补好配置后可直接重跑该 job。

### 3. 发版前检查

#### 3.1 commit 信息要符合 conventional commits

`git-cliff` 从最近一个 tag 到 HEAD 的提交里提取生成 changelog，识别以下前缀:

| 前缀 | 分组 | 是否出现在 changelog |
|---|---|---|
| `feat:` | Features | ✅ |
| `fix:` | Bug Fixes | ✅ |
| `perf:` | Performance | ✅ |
| `refactor:` | Refactor | ✅ |
| `docs:` | Documentation | ✅ |
| `chore:` | Miscellaneous | ❌ (skip) |
| `ci:` | CI/CD | ❌ (skip) |
| 其他 | — | ❌ (filter_unconventional) |

具体规则在 [cliff.toml](../cliff.toml) 里。如果发现 commit 没用正确前缀，在发版前用 `git commit --amend` 或 rebase 修正，否则不会出现在 release notes 里。

#### 3.2 本地确认 lint/test 全绿

CI 上的 `lint` job 是发版强依赖，本地先跑一遍能省一轮 CI 时间:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

#### 3.3 干跑一次 dry-run publish(可选)

CI 在每次 PR / push 都会跑 `publish-dry-run`，本地不是必须，但想提前发现 metadata 问题可以:

```bash
cargo publish --dry-run --locked --registry crates-io
```

### 4. 发版流程

#### 4.1 切换到发版分支

`cargo-release` 默认只允许从 `main` / `master` 发版。先合并 `dev` → `main`:

```bash
git checkout main
git pull
git merge --ff-only dev   # 或走 PR 合并到 main
```

> 如确需从 `dev` 直接发版，可在 `Cargo.toml` 的 `[package.metadata.release]` 加 `allow-branch = ["main", "dev"]`。

#### 4.2 干跑确认

不带 `--execute` 是 dry-run，会打印将要执行的所有动作但不真改:

```bash
cargo release patch          # 0.2.0 → 0.2.1
cargo release minor          # 0.2.0 → 0.3.0
cargo release 1.0.0          # 指定版本
```

确认预览列出以下计划动作（不会实际执行）:

- `Cargo.toml` 与 `Cargo.lock` 的 `version` 改写
- commit message: `release: <new-version>`
- tag: `v<new-version>`
- push to `origin`

#### 4.3 真正执行

```bash
cargo release patch --execute
```

执行后 `cargo-release` 会自动:

1. 改写 `Cargo.toml` + `Cargo.lock` 的 `version`
2. `git commit -m "release: <new-version>"`
3. `git tag -a v<new-version> -m "release <new-version>"`
4. `git push` + `git push --tags`

#### 4.4 等 CI 跑完

1. <https://github.com/maoXyzt/ssh-channels-hub/actions> 观察 `build` workflow
2. 顺序:`preflight` + `verify-tag` → `lint` → `build (×3)` + `wheels (×3)` + `changelog` → `release` + `publish` + `publish-pypi`
3. 全绿后:
   - GitHub Releases 出现 `v<new-version>`，挂着 3 份压缩包 + `.sha256`，body 是 git-cliff 渲染的 changelog
   - <https://crates.io/crates/ssh-channels-hub> 出现新版本
   - <https://pypi.org/project/ssh-channels-hub/> 出现新版本和 3 份 wheel

### 5. 防呆机制

发错版本的途径基本都被堵住:

| 风险 | 防御 |
|---|---|
| `Cargo.toml` 版本和 tag 不一致 | `cargo release` 同一动作产出两者；CI `publish` / `publish-pypi` 都依赖 `verify-tag` 再校验 tag↔manifest |
| `cargo release` 意外发布 | `[package.metadata.release].publish = false` 禁用其本地发布步骤；`[package].publish` 保持未设置，因此 CI 仍可运行 `cargo publish` |
| 平台编译挂掉但已发包 | `publish` 依赖 `build` 全绿；`publish-pypi` 依赖 `wheels` 全绿 |
| 代码有 fmt / clippy / 测试问题 | `lint` job 是 `release` / `publish` 的强依赖，失败则停发 |
| PR 引入 metadata 回归(license / readme / include) | 非 tag 推送都触发 `publish-dry-run` job 跑 `cargo publish --dry-run` |
| cargo-release 推 main + tag 触发双跑 CI | `preflight` job 识别 `release:` commit message，跳过 main 分支那一次重复构建 |
| PyPI token 泄漏 | PyPI 走 OIDC Trusted Publisher，没有长期 token |

### 6. 故障排查

#### `cargo release` 报 `branch 'dev' is not whitelisted`

发版分支不在 `allow-branch` 里。要么切回 `main`，要么按 §4.1 末尾说明改配置。

#### CI `publish` job 报 `crate version is already uploaded`

同一 `version` 在 crates.io 上已存在。crates.io 不允许覆盖，必须 bump 一个新版本号重发。本次的 `v<x.y.z>` tag 也建议 `git tag -d v<x.y.z> && git push --delete origin v<x.y.z>` 删掉，避免 GitHub Release 卡在半成品状态。

#### CI `verify-tag` job 失败

tag 名和 `Cargo.toml` 的 `version` 对不上。一般是手动 `git tag` 而绕过了 `cargo release` 导致。删 tag 重来:

```bash
git tag -d v<wrong>
git push --delete origin v<wrong>
cargo release <correct> --execute
```

#### Release notes 空了 / 缺少某个 commit

git-cliff 只识别 conventional commits 前缀(见 §3.1)。检查那段时间的 `git log --oneline v<prev>..HEAD`，把不符合前缀的 commit 找出来。已经推上去的 commit 改前缀代价大，通常的处理是在下一个版本的 release notes 里手动补一句说明，或者用 `git commit --allow-empty -m "fix: <说明>"` 补一条空 commit。

#### CI `lint` 因 clippy 新规则突然挂了

工具链 pin 在 `dtolnay/rust-toolchain@1.91`，clippy 规则不会无声跟着 stable 走。如果挂了，要么修代码，要么在 `Cargo.toml` 的 `[lints]` 或源码里加局部 `#[allow(...)]`。**不要** 把 `-D warnings` 改成 `-W warnings` 来绕过。

#### `publish-pypi` job 报 `invalid-publisher` / 401

检查 §2.3 的 project、owner、repository、workflow 和 environment 是否与 PyPI 后台完全一致。修正后重跑 `publish-pypi` job，无需重打 tag。

#### `publish-pypi` job 报 `File already exists`

PyPI 不允许覆盖同版本文件。必须 bump 新版本后重新发布。

### 7. 撤回已发版本

crates.io 不允许删除版本，只能 `yank`(标记为不推荐使用，新项目无法添加这个版本作依赖，已锁定的项目仍可下载):

```bash
cargo yank --version 0.2.1
cargo yank --version 0.2.1 --undo   # 反悔
```

PyPI 版本也不能覆盖；需要撤回时在 PyPI 版本页面执行 yank。

GitHub Release 可以直接在网页删除；git tag 用 `git push --delete origin v0.2.1` 移除。
