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
pip install demucs fastapi uvicorn[standard] python-multipart
sudo apt install -y ffmpeg
```

---

## 🚀 启动服务

```bash
uvicorn app:app --host 0.0.0.0 --port 8000
```

打开浏览器访问：

- Swagger UI：  
  👉 http://localhost:8000/docs

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
