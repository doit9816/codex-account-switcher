import { FileClock, FolderOpen, Save } from "lucide-react";
import type { I18n } from "../../i18n";

type RoutingLogSettingsProps = {
  t: I18n;
  retentionDays: number;
  busy: boolean;
  onRetentionDaysChange: (value: number) => void;
  onSave: () => void | Promise<unknown>;
  onOpenLogs: () => void | Promise<unknown>;
};

export function RoutingLogSettings({
  t,
  retentionDays,
  busy,
  onRetentionDaysChange,
  onSave,
  onOpenLogs
}: RoutingLogSettingsProps) {
  return (
    <section className="routing-log-settings-band">
      <div className="routing-log-settings-copy">
        <FileClock size={21} />
        <div>
          <h2>{t.routingLogSettings}</h2>
          <p>{t.routingLogRetentionHint}</p>
        </div>
      </div>
      <label>
        {t.routingLogRetention}
        <input
          className="small-number"
          type="number"
          min={1}
          max={365}
          value={retentionDays}
          onChange={(event) => onRetentionDaysChange(Number(event.target.value) || 1)}
          title={t.routingLogRetention}
        />
      </label>
      <button
        className="icon-button"
        onClick={() => void onSave()}
        disabled={busy}
        title={t.saveRoutingLogSettings}
      >
        <Save size={17} />
        {t.saveRoutingLogSettings}
      </button>
      <button
        className="icon-button"
        onClick={() => void onOpenLogs()}
        disabled={busy}
        title="打开日志目录"
      >
        <FolderOpen size={17} />
        打开日志目录
      </button>
    </section>
  );
}
