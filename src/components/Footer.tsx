import { useTranslation } from "react-i18next";
import { APP_VERSION } from "../lib/constants";

/**
 * Подвал — статичный текст с реквизитами протоколов и версией.
 */
export function Footer() {
  const { t } = useTranslation();
  return (
    <footer className="footer">
      <span>{t("footer.left")}</span>
      <span>© 2026 NEMEFISTO · v.{APP_VERSION}</span>
    </footer>
  );
}
