import { Button, Select, Tabs } from "antd";
import { LayoutDashboard, Network, RefreshCcw, Settings, Share2 } from "lucide-react";
import type { LanguageSetting } from "../types";

type Page = "dashboard" | "settings" | "mesh" | "routing";

type Props = {
  page: Page;
  onPageChange: (page: Page) => void;
  language: LanguageSetting;
  onLanguageChange: (language: LanguageSetting) => void;
  labels: { language: string; followSystem: string; dashboard: string; settings: string; refresh: string; refreshHint: string; subtitle: string; mesh: string; routing: string; page: string };
  languageLabels: Record<"zh-CN" | "en" | "zh-TW", string>;
  busy: boolean;
  onRefresh: () => void;
};

export function AppNavigation({ page, onPageChange, language, onLanguageChange, labels, languageLabels, busy, onRefresh }: Props) {
  return <header className="topbar">
    <div><h1>CodexSwitcher</h1><p>{labels.subtitle}</p></div>
    <div className="topbar-actions">
      <Select aria-label={labels.language} value={language} onChange={onLanguageChange} style={{ minWidth: 125 }} options={[
        { value: "system", label: labels.followSystem },
        { value: "zh-CN", label: languageLabels["zh-CN"] },
        { value: "en", label: languageLabels.en },
        { value: "zh-TW", label: languageLabels["zh-TW"] },
      ]} />
      <Tabs className="desktop-page-tabs" activeKey={page} onChange={(key) => onPageChange(key as Page)} items={[
        { key: "dashboard", label: <span title={labels.dashboard}><LayoutDashboard size={16} /> {labels.dashboard}</span> },
        { key: "settings", label: <span title={labels.settings}><Settings size={16} /> {labels.settings}</span> },
        { key: "mesh", label: <span title={labels.mesh}><Share2 size={16} /> {labels.mesh}</span> },
        { key: "routing", label: <span title={labels.routing}><Network size={16} /> {labels.routing}</span> },
      ]} />
      <Select
        className="mobile-page-select"
        aria-label={labels.page}
        value={page}
        onChange={(key) => onPageChange(key as Page)}
        options={[
          { value: "dashboard", label: labels.dashboard },
          { value: "settings", label: labels.settings },
          { value: "mesh", label: labels.mesh },
          { value: "routing", label: labels.routing },
        ]}
      />
      <Button type="primary" icon={<RefreshCcw size={16} />} disabled={busy} onClick={onRefresh} title={labels.refreshHint}>{labels.refresh}</Button>
    </div>
  </header>;
}
