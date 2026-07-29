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
    <fieldset className="profile-rule-section form-span-all">
      <legend>{t.accountRules}</legend>
      <div className="profile-rule-grid">
        <label>
          {t.hourlyLimit}
          <input
            type="number"
            min={0}
            placeholder={t.unlimited}
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
            placeholder={t.unlimited}
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
      <div className="profile-rule-footer">
        <label className="checkline profile-enabled-check" title={t.enableAccount}>
          <input
            type="checkbox"
            checked={enabled}
            onChange={(event) => onEnabledChange(event.target.checked)}
          />
          {t.enableAccount}
        </label>
      </div>
    </fieldset>
  );
}
