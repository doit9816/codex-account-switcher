import type { I18n } from "../i18n";
import type { QuotaRule } from "../types";
import { parseOptionalNumber } from "../profileUtils";

type ProfileRuleFieldsProps = {
  t: I18n;
  quota: QuotaRule;
  priority: number;
  enabled: boolean;
  onQuotaChange: (quota: QuotaRule) => void;
  onPriorityChange: (priority: number) => void;
  onEnabledChange: (enabled: boolean) => void;
};

export function ProfileRuleFields({
  t,
  quota,
  priority,
  enabled,
  onQuotaChange,
  onPriorityChange,
  onEnabledChange
}: ProfileRuleFieldsProps) {
  return (
    <>
      <div className="form-grid profile-rule-grid form-span-all">
        <label>
          {t.hourlyLimit}
          <input
            type="number"
            min={0}
            value={quota.hourlyLimit ?? ""}
            onChange={(event) => onQuotaChange({
              ...quota,
              hourlyLimit: parseOptionalNumber(event.target.value)
            })}
          />
        </label>
        <label>
          {t.dailyLimit}
          <input
            type="number"
            min={0}
            value={quota.dailyLimit ?? ""}
            onChange={(event) => onQuotaChange({
              ...quota,
              dailyLimit: parseOptionalNumber(event.target.value)
            })}
          />
        </label>
        <label>
          {t.cooldownMinutes}
          <input
            type="number"
            min={1}
            value={quota.cooldownMinutes}
            onChange={(event) => onQuotaChange({
              ...quota,
              cooldownMinutes: Number(event.target.value) || 180
            })}
          />
        </label>
        <label>
          {t.priority}
          <input
            type="number"
            value={priority}
            onChange={(event) => onPriorityChange(Number(event.target.value) || 0)}
          />
        </label>
      </div>
      <div className="switches form-span-all">
        <label className="checkline" title={t.enableAccount}>
          <input
            type="checkbox"
            checked={enabled}
            onChange={(event) => onEnabledChange(event.target.checked)}
          />
          {t.enableAccount}
        </label>
      </div>
    </>
  );
}
