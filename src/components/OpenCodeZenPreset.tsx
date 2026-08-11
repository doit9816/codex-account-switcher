import { Sparkles } from "lucide-react";
import { useState } from "react";

import type { I18n } from "../i18n";

export const OPENCODE_ZEN_BASE_URL = "https://opencode.ai/zen/v1";

export const OPENCODE_ZEN_FREE_MODELS = [
  { id: "mimo-v2.5-free", name: "MiMo-V2.5 Free" },
  { id: "deepseek-v4-flash-free", name: "DeepSeek V4 Flash Free" },
  { id: "longcat-2.0-free", name: "LongCat-2.0 Free" },
  { id: "nemotron-3-ultra-free", name: "Nemotron 3 Ultra Free" },
  { id: "north-mini-code-free", name: "North Mini Code Free" },
  { id: "laguna-s-2.1-free", name: "Laguna S 2.1 Free" },
  { id: "ling-3.0-tiny-free", name: "Ling-3.0-tiny Free" },
  { id: "big-pickle", name: "Big Pickle" },
] as const;

export type OpenCodeZenPresetValues = {
  providerName: string;
  baseUrl: string;
  model: string;
  wireApi: "chat_completions";
  apiKey: "public";
};

type OpenCodeZenPresetProps = {
  disabled?: boolean;
  onApply: (values: OpenCodeZenPresetValues) => void;
  t: I18n;
};

export function OpenCodeZenPreset({ disabled, onApply, t }: OpenCodeZenPresetProps) {
  const [model, setModel] = useState<string>(OPENCODE_ZEN_FREE_MODELS[0].id);

  function applyPreset() {
    onApply({
      providerName: `OpenCode Zen / ${model}`,
      baseUrl: OPENCODE_ZEN_BASE_URL,
      model,
      wireApi: "chat_completions",
      apiKey: "public",
    });
  }

  return (
    <section className="opencode-zen-preset">
      <div className="opencode-zen-preset-copy">
        <strong>{t.openCodeZenPresetTitle}</strong>
        <p>{t.openCodeZenPresetHint}</p>
      </div>
      <div className="opencode-zen-preset-actions">
        <label>
          {t.openCodeZenFreeModel}
          <select value={model} onChange={(event) => setModel(event.target.value)}>
            {OPENCODE_ZEN_FREE_MODELS.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name} ({item.id})
              </option>
            ))}
          </select>
        </label>
        <button className="icon-button" type="button" onClick={applyPreset} disabled={disabled}>
          <Sparkles size={16} /> {t.openCodeZenApplyPreset}
        </button>
      </div>
      <small>{t.openCodeZenApiKeyHint}</small>
    </section>
  );
}
