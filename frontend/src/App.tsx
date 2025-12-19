import { DragEvent, FormEvent, KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

const API_BASE_URL = import.meta.env.VITE_API_BASE_URL || "";
const ACCEPTED_FORMATS = ".mp3,.wav,.m4a,.flac,.ogg,.aac";
const SUPPORTED_FORMAT_LABEL = ACCEPTED_FORMATS.split(",")
  .map((ext) => ext.replace(".", "").toUpperCase())
  .join(" / ");

type JobResponse = {
  job_id: string;
  vocals_url: string;
  instrumental_url: string;
};

type Phase =
  | "idle"
  | "ready"
  | "uploading"
  | "processing"
  | "done"
  | "error_upload"
  | "error_processing";

type ProcessingStage = "starting" | "running";

type JobRequestError = {
  stage: "upload" | "processing";
  message: string;
  status?: number;
};

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return "-";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
}

export default function App() {
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const dragDepth = useRef(0);
  const processingTimerRef = useRef<number | null>(null);
  const bypassKeyInputRef = useRef<HTMLInputElement | null>(null);
  const keyAttentionTimerRef = useRef<number | null>(null);

  const [file, setFile] = useState<File | null>(null);
  const [bypassKey, setBypassKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [job, setJob] = useState<JobResponse | null>(null);
  const [dragging, setDragging] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [uploadProgress, setUploadProgress] = useState(0);
  const [uploadProgressKnown, setUploadProgressKnown] = useState(true);
  const [processingStage, setProcessingStage] = useState<ProcessingStage>("starting");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [focusBypassKey, setFocusBypassKey] = useState(false);
  const [keyAttention, setKeyAttention] = useState(false);
  const [lastErrorStatus, setLastErrorStatus] = useState<number | null>(null);

  const busy = phase === "uploading" || phase === "processing";

  const fileSummary = useMemo(() => {
    if (!file) return null;
    return `${file.name} · ${formatBytes(file.size)}`;
  }, [file]);

  const clearProcessingTimer = () => {
    if (processingTimerRef.current == null) return;
    window.clearTimeout(processingTimerRef.current);
    processingTimerRef.current = null;
  };

  const clearKeyAttentionTimer = () => {
    if (keyAttentionTimerRef.current == null) return;
    window.clearTimeout(keyAttentionTimerRef.current);
    keyAttentionTimerRef.current = null;
  };

  const requestKeyAttention = () => {
    clearKeyAttentionTimer();
    setKeyAttention(true);
    keyAttentionTimerRef.current = window.setTimeout(() => {
      setKeyAttention(false);
      keyAttentionTimerRef.current = null;
    }, 2200);
  };

  useEffect(() => {
    if (!advancedOpen || !focusBypassKey) return;
    setFocusBypassKey(false);
    window.requestAnimationFrame(() => {
      const input = bypassKeyInputRef.current;
      if (!input) return;
      input.focus();
      input.scrollIntoView({ behavior: "smooth", block: "center" });
    });
  }, [advancedOpen, focusBypassKey]);

  const clearFile = () => {
    if (busy) return;
    clearProcessingTimer();
    clearKeyAttentionTimer();
    setFile(null);
    setJob(null);
    setError(null);
    setLastErrorStatus(null);
    setPhase("idle");
    setUploadProgress(0);
    setUploadProgressKnown(true);
    setProcessingStage("starting");
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  const pickFile = () => {
    if (busy) return;
    fileInputRef.current?.click();
  };

  const onDragEnter = (event: DragEvent<HTMLDivElement>) => {
    if (busy) return;
    event.preventDefault();
    dragDepth.current += 1;
    setDragging(true);
  };

  const onDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (busy) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDragging(true);
  };

  const onDragLeave = (event: DragEvent<HTMLDivElement>) => {
    if (busy) return;
    event.preventDefault();
    dragDepth.current -= 1;
    if (dragDepth.current <= 0) {
      dragDepth.current = 0;
      setDragging(false);
    }
  };

  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    if (busy) return;
    event.preventDefault();
    dragDepth.current = 0;
    setDragging(false);
    const dropped = event.dataTransfer.files?.[0];
    if (!dropped) return;
    setFile(dropped);
    setError(null);
    setJob(null);
    setPhase("ready");
  };

  const onDropzoneKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (busy) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    pickFile();
  };

  const getErrorMessage = (text: string, status: number): string => {
    const trimmed = text.trim();
    if (!trimmed) return `请求失败（HTTP ${status}）`;
    try {
      const json = JSON.parse(trimmed) as unknown;
      if (json && typeof json === "object" && "detail" in json) {
        return String((json as { detail?: unknown }).detail);
      }
    } catch {
      // ignore
    }
    return trimmed;
  };

  const uploadAndCreateJob = (uploadFile: File): Promise<JobResponse> =>
    new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      let uploadFinished = false;

      xhr.open("POST", `${API_BASE_URL}/api/jobs`);
      xhr.responseType = "text";
      if (bypassKey.trim()) {
        xhr.setRequestHeader("X-VS-Bypass-Key", bypassKey.trim());
      }

      xhr.upload.addEventListener("progress", (event) => {
        if (uploadFinished) return;
        if (event.lengthComputable) {
          setUploadProgressKnown(true);
          const percent = Math.max(0, Math.min(100, Math.round((event.loaded / event.total) * 100)));
          setUploadProgress(percent);
        } else {
          setUploadProgressKnown(false);
          setUploadProgress(0);
        }
      });

      xhr.upload.addEventListener("load", () => {
        uploadFinished = true;
        setUploadProgressKnown(true);
        setUploadProgress(100);
        setPhase("processing");
        setProcessingStage("starting");
        clearProcessingTimer();
        processingTimerRef.current = window.setTimeout(() => {
          setProcessingStage("running");
          processingTimerRef.current = null;
        }, 900);
      });

      xhr.addEventListener("load", () => {
        const ok = xhr.status >= 200 && xhr.status < 300;
        if (!ok) {
          const message = getErrorMessage(xhr.responseText || "", xhr.status);
          const stage: JobRequestError["stage"] = xhr.status >= 500 ? "processing" : "upload";
          reject({
            stage,
            message,
            status: xhr.status,
          } satisfies JobRequestError);
          return;
        }

        let json: unknown;
        try {
          json = JSON.parse(xhr.responseText || "");
        } catch {
          reject({
            stage: "processing",
            message: "响应解析失败",
          } satisfies JobRequestError);
          return;
        }

        if (!json || typeof json !== "object") {
          reject({
            stage: "processing",
            message: "响应格式不正确",
          } satisfies JobRequestError);
          return;
        }

        const payload = json as Partial<JobResponse>;
        if (
          typeof payload.job_id !== "string" ||
          typeof payload.vocals_url !== "string" ||
          typeof payload.instrumental_url !== "string"
        ) {
          reject({
            stage: "processing",
            message: "响应格式不正确",
          } satisfies JobRequestError);
          return;
        }

        resolve(payload as JobResponse);
      });

      xhr.addEventListener("error", () => {
        reject({
          stage: uploadFinished ? "processing" : "upload",
          message: "网络错误",
          status: undefined,
        } satisfies JobRequestError);
      });

      xhr.addEventListener("abort", () => {
        reject({
          stage: uploadFinished ? "processing" : "upload",
          message: "已取消",
          status: undefined,
        } satisfies JobRequestError);
      });

      const formData = new FormData();
      formData.append("file", uploadFile);
      xhr.send(formData);
    });

  const startJob = async () => {
    if (busy) return;
    if (!file) {
      setError("请选择一个音频文件");
      setPhase("idle");
      return;
    }
    setError(null);
    setLastErrorStatus(null);
    setJob(null);
    setUploadProgressKnown(true);
    setUploadProgress(0);
    setProcessingStage("starting");
    clearProcessingTimer();
    setPhase("uploading");

    try {
      const data = await uploadAndCreateJob(file);
      setJob(data);
      clearProcessingTimer();
      setPhase("done");
    } catch (err) {
      clearProcessingTimer();
      const status =
        err && typeof err === "object" && "status" in err ? Number((err as JobRequestError).status) : null;
      setLastErrorStatus(Number.isFinite(status) ? status : null);
      if (status === 429) {
        setAdvancedOpen(true);
        setFocusBypassKey(true);
        requestKeyAttention();
      }
      const message =
        err && typeof err === "object" && "message" in err ? String((err as JobRequestError).message) : "请求失败";
      const stage =
        err && typeof err === "object" && "stage" in err ? (err as JobRequestError).stage : "upload";
      setError(message);
      setPhase(stage === "processing" ? "error_processing" : "error_upload");
    }
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    await startJob();
  };

  const statusText = (() => {
    switch (phase) {
      case "idle":
        return "选择音频文件开始";
      case "ready":
        return "已选择文件，点击开始分离";
      case "uploading":
        return uploadProgressKnown ? `上传中 ${uploadProgress}%` : "上传中…";
      case "processing":
        return processingStage === "starting" ? "上传成功，开始解析…" : "解析中…";
      case "done":
        return "解析完成";
      case "error_upload":
        return "上传失败";
      case "error_processing":
        return "解析失败";
    }
  })();

  const stepState = (step: 1 | 2 | 3 | 4): "todo" | "active" | "done" | "error" => {
    if (step === 1) {
      if (!file) return "active";
      return "done";
    }

    if (step === 2) {
      if (!file) return "todo";
      if (phase === "uploading") return "active";
      if (phase === "error_upload") return "error";
      if (phase === "processing" || phase === "done" || phase === "error_processing") return "done";
      return "todo";
    }

    if (step === 3) {
      if (phase === "processing") return "active";
      if (phase === "error_processing") return "error";
      if (phase === "done") return "done";
      return "todo";
    }

    if (step === 4) {
      if (phase === "done") return "done";
      return "todo";
    }

    return "todo";
  };

  const stepIcon = (step: 1 | 2 | 3 | 4): string => {
    const state = stepState(step);
    if (state === "done") return "✓";
    if (state === "error") return "!";
    return String(step);
  };

  const dropzoneClassName = (() => {
    const classes: string[] = ["dropzone"];
    if (dragging) classes.push("dragging");
    if (file) classes.push("hasFile");
    if (busy) classes.push("busy");
    if (phase === "error_upload" || phase === "error_processing") classes.push("error");
    return classes.join(" ");
  })();

  return (
    <div className="app">
      <header className="hero">
        <div className="heroMain">
          <div className="titleRow">
            <h1 className="title">Vocal Separator</h1>
            <span className="tag">Beta</span>
          </div>
          <p className="subtitle">上传歌曲，一键分离人声与伴奏（Demucs）</p>
        </div>
      </header>

      <main className="grid">
        <section className="card workCard">
          <div className="cardHeader">
            <h2 className="cardTitle">上传并分离</h2>
            <p className="cardDesc">支持 {SUPPORTED_FORMAT_LABEL}</p>
          </div>

          <div className="workflow">
            <div className="stepper" aria-label="处理步骤">
              <div className={`step ${stepState(1)}`}>
                <span className="stepDot" aria-hidden="true">
                  {stepIcon(1)}
                </span>
                <span className="stepLabel">选择文件</span>
              </div>
              <div className={`step ${stepState(2)}`}>
                <span className="stepDot" aria-hidden="true">
                  {stepIcon(2)}
                </span>
                <span className="stepLabel">上传</span>
              </div>
              <div className={`step ${stepState(3)}`}>
                <span className="stepDot" aria-hidden="true">
                  {stepIcon(3)}
                </span>
                <span className="stepLabel">解析</span>
              </div>
              <div className={`step ${stepState(4)}`}>
                <span className="stepDot" aria-hidden="true">
                  {stepIcon(4)}
                </span>
                <span className="stepLabel">完成</span>
              </div>
            </div>

            <div className="statusRow">
              <div
                className={
                  phase === "done"
                    ? "statusPill success"
                    : phase === "error_upload" || phase === "error_processing"
                      ? "statusPill danger"
                      : "statusPill"
                }
              >
                {phase === "processing" && <span className="spinner" aria-hidden="true" />}
                <span>{statusText}</span>
              </div>
              {phase === "uploading" && uploadProgressKnown && (
                <div className="statusMeta" aria-label="上传进度">
                  {uploadProgress}%
                </div>
              )}
            </div>

            {(phase === "uploading" || phase === "processing") && (
              <div className="progressWrap" aria-label="进度">
                <div
                  className={
                    phase === "processing" || !uploadProgressKnown
                      ? "progressBar indeterminate"
                      : "progressBar"
                  }
                >
                  <div
                    className="progressFill"
                    style={
                      phase === "uploading" && uploadProgressKnown ? { width: `${uploadProgress}%` } : undefined
                    }
                  />
                </div>
              </div>
            )}
          </div>

          <form onSubmit={handleSubmit}>
            <div
              className={dropzoneClassName}
              role="button"
              tabIndex={busy ? -1 : 0}
              aria-label="选择音频文件"
              aria-disabled={busy}
              onClick={pickFile}
              onKeyDown={onDropzoneKeyDown}
              onDragEnter={onDragEnter}
              onDragOver={onDragOver}
              onDragLeave={onDragLeave}
              onDrop={onDrop}
            >
              <input
                ref={fileInputRef}
                className="fileInput"
                type="file"
                accept={ACCEPTED_FORMATS}
                disabled={busy}
                onChange={(e) => {
                  const next = e.target.files?.[0] ?? null;
                  setFile(next);
                  setError(null);
                  setJob(null);
                  setPhase(next ? "ready" : "idle");
                }}
              />
              <div className="dropzoneBody">
                <div className="dropzoneTitle">
                  {file ? "已选择文件" : "拖拽音频到这里，或点击选择"}
                </div>
                <div className="dropzoneMeta">
                  {fileSummary ? fileSummary : `格式：${SUPPORTED_FORMAT_LABEL}`}
                </div>
              </div>
              <div className="dropzoneActions" aria-hidden={!file}>
                {file && (
                  <>
                    <button
                      type="button"
                      className="ghostBtn"
                      disabled={busy}
                      onClick={(e) => {
                        e.stopPropagation();
                        pickFile();
                      }}
                    >
                      更换
                    </button>
                    <button
                      type="button"
                      className="ghostBtn danger"
                      disabled={busy}
                      onClick={(e) => {
                        e.stopPropagation();
                        clearFile();
                      }}
                    >
                      清除
                    </button>
                  </>
                )}
              </div>
            </div>

            <details
              className={keyAttention ? "advanced attention" : "advanced"}
              open={advancedOpen}
              onToggle={(e) => setAdvancedOpen(e.currentTarget.open)}
            >
              <summary className="advancedSummary">高级选项（Key）</summary>
              <div className="advancedBody">
                <div className="field">
                  <div className="fieldLabelRow">
                    <span className="fieldLabel">Key（可选）</span>
                    <span className="fieldHint">填写后可无限制使用</span>
                  </div>
                  {lastErrorStatus === 429 && (
                    <div className="keyCallout" role="note">
                      检测到今日次数已用完：输入 Key 可继续使用。
                    </div>
                  )}
                  <input
                    ref={bypassKeyInputRef}
                    className="textInput"
                    type="password"
                    inputMode="text"
                    autoComplete="off"
                    spellCheck={false}
                    value={bypassKey}
                    onChange={(e) => setBypassKey(e.target.value)}
                    disabled={busy}
                    placeholder="输入 Key"
                  />
                  <div className="keyHelp">
                    没有 Key？联系{" "}
                    <a className="keyLink" href="mailto:swartz_lubel@outlook.com">
                      swartz_lubel@outlook.com
                    </a>
                  </div>
                </div>
              </div>
            </details>

            <button type="submit" className="primaryBtn" disabled={!file || busy || phase === "done"}>
              {phase === "uploading"
                ? "上传中..."
                : phase === "processing"
                  ? "解析中..."
                  : phase === "done"
                    ? "已完成"
                    : phase === "error_upload" || phase === "error_processing"
                      ? "重试"
                      : "开始分离"}
            </button>
          </form>

          {error && (
            <div className="alert errorAlert" role="alert">
              <div className="alertTitle">
                {lastErrorStatus === 429
                  ? "今日已达上限"
                  : phase === "error_processing"
                    ? "解析失败"
                    : phase === "error_upload"
                      ? "上传失败"
                      : "处理失败"}
              </div>
              <div className="alertMsg">{error}</div>
              {file && !busy && (phase === "error_upload" || phase === "error_processing") && (
                <div className="alertActions">
                  <button type="button" className="ghostBtn" onClick={() => void startJob()}>
                    重试
                  </button>
                  <button type="button" className="ghostBtn danger" onClick={clearFile}>
                    清除文件
                  </button>
                </div>
              )}
            </div>
          )}

          {job && (
            <div className="result">
              <div className="resultTop">
                <div className="resultTitle">分离完成</div>
              </div>
              <div className="resultActions">
                <a
                  className="downloadBtn primary"
                  href={`${API_BASE_URL}${job.vocals_url}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  下载人声
                </a>
                <a
                  className="downloadBtn"
                  href={`${API_BASE_URL}${job.instrumental_url}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  下载伴奏
                </a>
              </div>
            </div>
          )}
        </section>

        <aside className="card">
          <div className="cardHeader">
            <h2 className="cardTitle">小贴士</h2>
            <p className="cardDesc">更快、更稳、更好听</p>
          </div>
          <ul className="tips">
            <li>建议上传 30 秒以上的音频，效果更稳定。</li>
            <li>如果是大文件，分离时间会更长，请耐心等待。</li>
            <li>输出包含人声（vocals）与伴奏（instrumental）。</li>
          </ul>
          <div className="divider" />
          <div className="mini">
            <div className="miniTitle">隐私</div>
            <div className="miniDesc">文件仅用于分离处理；请勿上传敏感内容。</div>
          </div>
        </aside>
      </main>
    </div>
  );
}
