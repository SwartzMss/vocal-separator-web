import { FormEvent, useState } from "react";

const API_BASE_URL = import.meta.env.VITE_API_BASE_URL || "";

type JobResponse = {
  job_id: string;
  vocals_url: string;
  instrumental_url: string;
};

export default function App() {
  const [file, setFile] = useState<File | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [job, setJob] = useState<JobResponse | null>(null);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!file) {
      setError("请选择一个音频文件");
      return;
    }
    setLoading(true);
    setError(null);
    setJob(null);

    const formData = new FormData();
    formData.append("file", file);

    try {
      const response = await fetch(`${API_BASE_URL}/api/jobs`, {
        method: "POST",
        body: formData,
      });
      if (!response.ok) {
        throw new Error(await response.text());
      }
      const data: JobResponse = await response.json();
      setJob(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "上传失败");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="app">
      <h1>Vocal Separator</h1>
      <div className="card">
        <form onSubmit={handleSubmit}>
          <label>
            选择音频文件
            <input
              type="file"
              accept=".mp3,.wav,.m4a,.flac,.ogg,.aac"
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            />
          </label>
          <button type="submit" disabled={!file || loading}>
            {loading ? "分离中..." : "开始分离"}
          </button>
        </form>

        {error && <p className="error">{error}</p>}

        {job && (
          <div className="status">
            <p>Job ID: {job.job_id}</p>
            <div className="links">
              <a href={`${API_BASE_URL}${job.vocals_url}`} target="_blank" rel="noreferrer">
                下载人声
              </a>
              <a
                href={`${API_BASE_URL}${job.instrumental_url}`}
                target="_blank"
                rel="noreferrer"
              >
                下载伴奏
              </a>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
