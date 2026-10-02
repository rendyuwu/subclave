import { createRoot } from "react-dom/client";
import { PopupApp } from "./PopupApp";
import "../styles/popup.css";

const container = document.getElementById("root");
if (container) createRoot(container).render(<PopupApp />);
