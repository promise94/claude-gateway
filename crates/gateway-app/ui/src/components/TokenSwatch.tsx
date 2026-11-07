import { colorTokens, type ColorToken } from "../lib/tokens";

interface TokenSwatchProps {
  name: ColorToken;
  label: string;
}

export function TokenSwatch({ name, label }: TokenSwatchProps) {
  return (
    <div className="cg-swatch">
      <span
        className="cg-swatch__chip"
        style={{ background: colorTokens[name] }}
        aria-hidden="true"
      />
      <span className="cg-swatch__label">{label}</span>
      <code className="cg-swatch__value">{colorTokens[name]}</code>
    </div>
  );
}
