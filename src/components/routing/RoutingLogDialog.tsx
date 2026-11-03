import { Modal } from "antd";
import { formatDate } from "../../profileUtils";
import type { RoutingLogEntry } from "../../types";
import { routingLogResult, routingLogTone } from "./routingView";

type RoutingLogDialogProps = {
  log: RoutingLogEntry;
  onClose: () => void;
};

export function RoutingLogDialog({ log, onClose }: RoutingLogDialogProps) {
  return (
    <Modal
      open
      onCancel={onClose}
      footer={null}
      title={<span id="routing-log-title">请求详情 · {formatDate(log.ts)}</span>}
      width={760}
      aria-labelledby="routing-log-title"
      styles={{ body: { maxHeight: "calc(100vh - 180px)", overflow: "auto", padding: 4 } }}
    >
        <dl className="routing-log-detail-grid">
          <div><dt>请求 ID</dt><dd>{log.requestId || "-"}</dd></div>
          <div><dt>入口</dt><dd>{log.method || "POST"} {log.path || "/v1/responses"}</dd></div>
          <div className={routingLogTone(log) === "error" ? "detail-error" : ""}><dt>结果</dt><dd>{routingLogResult(log)}</dd></div>
          <div><dt>HTTP 状态</dt><dd>{log.httpStatus || "-"}</dd></div>
          <div><dt>耗时</dt><dd>{log.latencyMs} ms</dd></div>
          <div><dt>命中账号</dt><dd>{log.alias || log.profileId || "-"}</dd></div>
          <div><dt>Profile ID</dt><dd>{log.profileId || "-"}</dd></div>
          <div><dt>请求模型</dt><dd>{log.requestedModel || "-"}</dd></div>
          <div><dt>实际模型</dt><dd>{log.actualModel || "-"}</dd></div>
          <div><dt>上游协议</dt><dd>{log.wireProtocol || "-"}</dd></div>
          <div className="detail-span-all"><dt>上游地址</dt><dd>{log.upstreamUrl || "-"}</dd></div>
          <div><dt>会话哈希</dt><dd>{log.sessionHash || "-"}</dd></div>
          <div><dt>回退原因（前一上游）</dt><dd>{log.fallback || "-"}</dd></div>
          <div className={`detail-span-all ${log.error ? "detail-error" : ""}`}><dt>错误</dt><dd>{log.error || "无"}</dd></div>
        </dl>
        <p className="routing-log-privacy">为保护隐私，请求提示词、响应正文和密钥不会写入日志。</p>
    </Modal>
  );
}
