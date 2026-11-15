import { Button, Card } from "antd";
import { Github, Star } from "lucide-react";
import type { I18n } from "../i18n";

type GitHubStarCardProps = {
  t: I18n;
  busy: boolean;
  onOpen: () => void | Promise<unknown>;
};

export function GitHubStarCard({ t, busy, onOpen }: GitHubStarCardProps) {
  return (
    <Card className="settings-band github-star-band">
      <div className="github-star-copy">
        <span className="github-star-icon" aria-hidden="true">
          <Github size={22} />
        </span>
        <div>
          <h2>{t.githubStarTitle}</h2>
          <p>{t.githubStarHint}</p>
        </div>
      </div>
      <Button
        type="primary"
        icon={<Star size={17} />}
        onClick={() => void onOpen()}
        disabled={busy}
        title={t.githubStarButtonHint}
        aria-label={t.githubStarButtonHint}
      >
        {t.githubStarButton}
      </Button>
    </Card>
  );
}
