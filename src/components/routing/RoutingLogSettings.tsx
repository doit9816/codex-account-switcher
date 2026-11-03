import { FileClock, FolderOpen, Save } from "lucide-react";
import type { I18n } from "../../i18n";
import { Button, InputNumber, Card } from "antd";

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
    <Card className="settings-band routing-log-settings-band">
      <div className="routing-log-settings-copy">
        <FileClock size={21} />
        <div>
          <h2>{t.routingLogSettings}</h2>
          <p>{t.routingLogRetentionHint}</p>
        </div>
      </div>
      <label>
        {t.routingLogRetention}
        <InputNumber
          min={1}
          max={365}
          value={retentionDays}
          onChange={(value) => onRetentionDaysChange(value || 1)}
          title={t.routingLogRetention}
          style={{ width: 100 }}
        />
      </label>
      <Button
        onClick={() => void onSave()}
        disabled={busy}
        title={t.saveRoutingLogSettings}
        icon={<Save size={17} />}
      >
        {t.saveRoutingLogSettings}
      </Button>
      <Button
        onClick={() => void onOpenLogs()}
        disabled={busy}
        title="打开日志目录"
        icon={<FolderOpen size={17} />}
      >
        打开日志目录
      </Button>
    </Card>
  );
}
