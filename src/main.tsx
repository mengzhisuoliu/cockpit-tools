import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initI18n } from "./i18n";
import { AppRuntimeGuard } from "./components/AppRuntimeGuard";
import { applyCachedStartupAppearance } from "./utils/startupAppearance";
import { startUiHangDiagnostics } from "./utils/hangDiagnostics";

applyCachedStartupAppearance();
startUiHangDiagnostics();
void initI18n();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <AppRuntimeGuard>
      <App />
    </AppRuntimeGuard>
  </React.StrictMode>,
);
