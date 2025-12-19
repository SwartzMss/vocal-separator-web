# vocal-separator-web

基于 **Demucs + PyTorch (CUDA)** 的 Web 版人声 / 伴奏分离服务。  
支持在 **Windows + WSL2 + NVIDIA GPU** 环境下运行，提供简单易用的 HTTP API，用于将一首歌曲拆分为 **人声（vocals）** 和 **伴奏（instrumental）**。

---

## ✨ 功能特性

- 🎵 人声 / 伴奏分离（`vocals.wav` / `no_vocals.wav`）
- ⚡ GPU 加速（CUDA / RTX 3090 实测秒级）
- 🌐 Web API（FastAPI）
- 🧱 并发限制，避免 GPU OOM
- 📦 支持多种音频格式（mp3 / wav / m4a / flac / ogg / aac）

---

## 🧠 技术栈

- **Demucs**：音频源分离模型
- **PyTorch (CUDA)**：GPU 推理
- **FastAPI**：Web API
- **FFmpeg**：音频解码
- **Python 3.10+**
- **WSL2 + NVIDIA GPU**

---

## 🖥️ 运行环境要求

### 操作系统
- Windows 10 / 11
- WSL2（Ubuntu 推荐）

### GPU
- NVIDIA GPU（已验证：RTX 3090）
- Windows NVIDIA 驱动（支持 WSL）
- WSL 中 `nvidia-smi` 可用

### Python
- Python 3.10+
- 已安装 **GPU 版 PyTorch**

---

## 🔧 环境准备

### 1️⃣ 创建虚拟环境（推荐）
```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -U pip
```

### 2️⃣ 安装 PyTorch（GPU 版）
```bash
pip install torch torchvision torchaudio \
  --index-url https://download.pytorch.org/whl/cu124
```

验证：
```bash
python - <<'PY'
import torch
print(torch.cuda.is_available())
print(torch.cuda.get_device_name(0))
PY
```

---

### 3️⃣ 安装依赖
```bash
pip install -r requirements.txt
sudo apt install -y ffmpeg
```

> ⚠️ `requirements.txt` 不会自动安装 GPU 版 PyTorch，请按上一步的指引单独安装。

---

## 🚀 启动服务

```bash
uvicorn app:app --host 0.0.0.0 --port 8000
```

可用环境变量：

- `MAX_CONCURRENT_JOBS`：允许的并发任务数量（默认 `1`）
- `DEMUCS_MODEL`：Demucs 模型名称（默认 `mdx_extra_q`）
- `DEMUCS_DEVICE`：运行设备（默认 `cuda`）

打开浏览器访问：

- Swagger UI：  
  👉 http://localhost:8000/docs

---

## 🛠 CLI 使用

无需启动 Web 服务即可在本地生成结果：

```bash
python cli.py /path/to/song.mp3 -o outputs
```

- 输入支持 `.mp3/.wav/.m4a/.flac/.ogg/.aac`
- 输出目录默认为 `outputs/`，会生成 `xxx_vocals.wav` / `xxx_instrumental.wav`
- CLI 同样读取 `DEMUCS_MODEL` / `DEMUCS_DEVICE` 环境变量

---

## 🤖 Python Agent（子进程模式）

给 Rust / 其他后端调用的无头版本：

```bash
python agent.py --input /tmp/song.mp3 --output-dir /tmp/job123
```

- 输出目录中会生成 `vocals.wav` 与 `instrumental.wav`
- 成功时 stdout 返回 `{"vocals": "...", "instrumental": "..."}` JSON
- 失败时 stderr 输出 error JSON，退出码非 0
- 同样支持 `DEMUCS_MODEL` / `DEMUCS_DEVICE`

---

## 🦀 Rust 后端（调度 + API）

Rust 负责 HTTP / 并发 / Agent 调用，Python 仅做推理：

```bash
cd backend
cargo run
# 或使用 release 构建
cargo run --release
```

默认监听 `0.0.0.0:8000`，并会调用仓库根目录的 `agent.py`。可用环境变量：

- `HOST` / `PORT`：监听地址与端口（默认 `0.0.0.0` / `8000`）
- `JOBS_DIR`：任务输出目录（默认 `../jobs`，即仓库根目录 `jobs/`）
- `AGENT_SCRIPT`：Python agent 路径（默认 `../agent.py`，即仓库根目录 `agent.py`）
- `PYTHON_BIN`：Python 可执行文件名（默认 `python3`）
- `LOG_DIR`：后端与 agent 写入日志的目录（默认 `../logs`，即仓库根目录 `logs/`，会生成 `backend.log` / `agent.log`）
- `DAILY_LIMIT_PER_BROWSER`：每个浏览器每日允许使用次数（默认 `0` 不限制；设置为 `1` 表示每天一次，基于 cookie `vs_bid`）
- `BYPASS_KEY`：绕过每日限制的 Key（前端可在“高级选项”输入，或请求头携带 `X-VS-Bypass-Key`）
- `JOBS_TTL_SECONDS`：job 结果缓存保留时间（秒，默认 `3600`；`0` 表示不自动删除）
- `JOBS_CLEANUP_INTERVAL_SECONDS`：后台清理扫描间隔（秒，默认 `600`）

Rust 后端提供的 API 与 FastAPI 版本保持一致，可以直接被前端或第三方服务调用。

---

## ⚛️ React 前端

`frontend/` 目录提供了基于 Vite + React 的简单上传页：

```bash
cd frontend
npm install        # 首次需要联网下载依赖
npm run dev        # 默认 5173 端口
```

- 默认把请求发送到同域 `/api`，如果前端单独部署，可设置：
  - `VITE_API_BASE_URL`：API 完整地址（如 `http://localhost:8000`）
  - `VITE_API_PROXY`：开发模式下的代理地址，便于解决 CORS
- 构建：`npm run build`，产物位于 `frontend/dist/`

> 本地若无法访问 npm registry，请先配置网络代理或使用离线包再运行 `npm install`。

---

## 🛠 部署脚本

`scripts/` 目录内提供了 Rain 项目的通用脚本，已适配本仓库：

- `scripts/build.sh`：校验 `cargo`/`npm`/`python3`/`pip` 后执行 `cargo clippy`、`cargo build --release`、`npm run build`，并自动执行 `pip install -r requirements.txt`（全局环境）。
- `scripts/deploy.sh`：需要 `sudo`，集成构建、静态资源同步、systemd 单元与 nginx 配置。默认服务名 `vocal-separator-web`，静态目录 `/var/www/vocal-separator-web`。服务进程默认以 `sudo` 发起者身份运行（可通过 `.env` 设置 `SERVICE_USER` 覆盖），其余路径/端口/证书信息也都来自 `.env`。

在首次部署前，复制 `backend/.env.example` 为 `backend/.env` 并按需修改证书路径、域名、监听地址等配置：

```bash
cp backend/.env.example backend/.env
vim backend/.env
```

如果上传的音频较大，可修改 `.env` 中的 `CLIENT_MAX_BODY_SIZE`（默认 `200M`），部署脚本会自动把它写入 nginx `client_max_body_size`。同样地，通过 `JOBS_DIR` / `LOG_DIR` / `AGENT_SCRIPT` 可以把上传与日志目录指向仓库（默认值已经是 `../jobs`、`../logs`、`../agent.py`，以 WorkingDirectory `backend/` 为基准）。

部署流程示例：

```bash
sudo scripts/deploy.sh install   # 首次安装
sudo scripts/deploy.sh restart   # 更新部署
sudo scripts/deploy.sh status    # 查看 systemd 状态
sudo scripts/deploy.sh uninstall # 停止服务并移除 nginx/systemd/静态文件
```

> 运行脚本前请确保目标机器已安装 `cargo`、`npm`、`rsync`、`nginx`、`systemd`。

部署完成后：

- Rust 后端日志写入 `${LOG_DIR}/backend.log`（同时也会打印到 `journalctl`）
- Python agent 日志写入 `${AGENT_LOG_FILE}`，默认 `${LOG_DIR}/agent.log`

---

## 📡 API 使用说明

### `POST /api/jobs`
上传音频并进行分离（同步执行）

**请求**
- `multipart/form-data`
- 字段名：`file`

**支持格式**
```
.mp3 .wav .m4a .flac .ogg .aac
```

**响应示例**
```json
{
  "job_id": "c9e9e8a7-xxxx-xxxx-xxxx-xxxxxxxx",
  "instrumental_url": "/api/jobs/{job_id}/instrumental",
  "vocals_url": "/api/jobs/{job_id}/vocals"
}
```

---

### `GET /api/jobs/{job_id}/instrumental`
下载伴奏（no vocals）

### `GET /api/jobs/{job_id}/vocals`
下载人声

---

## ⚙️ GPU 与并发说明

- Demucs 使用 `-d cuda` 强制走 GPU
- 默认 **同一时间仅允许 1 个任务运行**
- 可在代码中调整 semaphore 数量（⚠️ 注意显存占用）

---

## ⚠️ 已知限制

- 当前版本为 **同步执行**
  - 上传后需等待分离完成
- 未实现任务队列 / 进度查询
- 未做用户鉴权与限流

---

## 🗺️ Roadmap

- [ ] 异步任务队列（job 状态：queued / running / done）
- [ ] 前端页面（上传 + 进度 + 下载）
- [ ] Docker / docker-compose
- [ ] 多模型支持（4-stems）
- [ ] API 访问限流 / 鉴权

---

## 📜 License

MIT License

---

## 🙌 致谢

- Meta AI - Demucs  
- PyTorch 社区  
- FastAPI
