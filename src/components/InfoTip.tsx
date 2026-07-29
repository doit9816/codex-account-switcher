import { CircleHelp } from "lucide-react";

type InfoTipProps = {
  text: string;
};

export function InfoTip({ text }: InfoTipProps) {
  return (
    <span className="info-tip" tabIndex={0} aria-label={text}>
      <CircleHelp size={14} aria-hidden="true" />
      <span className="info-tip-content" role="tooltip">{text}</span>
    </span>
  );
}
