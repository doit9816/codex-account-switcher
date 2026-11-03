import type { I18n } from "../i18n";
import { Checkbox, InputNumber } from "antd";
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
          <InputNumber
            min={0}
            placeholder={t.unlimited}
            value={quota.hourlyLimit ?? null}
            onChange={(value) => onQuotaChange({
              ...quota,
              hourlyLimit: parseOptionalNumber(value == null ? "" : String(value))
            })}
            style={{ width: "100%" }}
          />
        </label>
        <label>
          {t.dailyLimit}
          <InputNumber
            min={0}
            placeholder={t.unlimited}
            value={quota.dailyLimit ?? null}
            onChange={(value) => onQuotaChange({
              ...quota,
              dailyLimit: parseOptionalNumber(value == null ? "" : String(value))
            })}
            style={{ width: "100%" }}
          />
        </label>
        <label>
          {t.cooldownMinutes}
          <InputNumber
            min={1}
            value={quota.cooldownMinutes}
            onChange={(value) => onQuotaChange({
              ...quota,
              cooldownMinutes: value || 180
            })}
            style={{ width: "100%" }}
          />
        </label>
        <label>
          {t.priority}
          <InputNumber
            value={priority}
            onChange={(value) => onPriorityChange(value || 0)}
            style={{ width: "100%" }}
          />
        </label>
      </div>
      <div className="profile-rule-footer">
        <label className="checkline profile-enabled-check" title={t.enableAccount}>
          <Checkbox
            checked={enabled}
            onChange={(event) => onEnabledChange(event.target.checked)}
          >{t.enableAccount}</Checkbox>
        </label>
      </div>
    </fieldset>
  );
}
