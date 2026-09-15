export type ToastTone = "success" | "warning" | "error";
export type StatusTone = "ready" | "attention" | "busy";
export type ToastHandler = (message: string, tone?: ToastTone) => void;
