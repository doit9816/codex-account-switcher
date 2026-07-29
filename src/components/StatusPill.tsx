type StatusPillProps = {
  ok: boolean;
  text: string;
};

export function StatusPill({ ok, text }: StatusPillProps) {
  return <span className={`status-pill ${ok ? "ok" : "muted"}`}>{text}</span>;
}
