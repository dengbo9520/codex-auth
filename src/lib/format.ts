import {
  format,
  formatDistanceToNowStrict,
  isValid,
} from "date-fns";
import { zhCN } from "date-fns/locale";

export function formatTimestamp(timestampMs: number | null | undefined) {
  if (!timestampMs) {
    return "暂无";
  }

  const date = new Date(timestampMs);
  if (!isValid(date)) {
    return "暂无";
  }

  return format(date, "yyyy-MM-dd HH:mm:ss");
}

export function formatRelative(timestampMs: number | null | undefined) {
  if (!timestampMs) {
    return "暂无";
  }

  const date = new Date(timestampMs);
  if (!isValid(date)) {
    return "暂无";
  }

  return `${formatDistanceToNowStrict(date, { addSuffix: true, locale: zhCN })}`;
}

export function formatShortTimestamp(timestampMs: number | null | undefined) {
  if (!timestampMs) {
    return "暂无";
  }

  const date = new Date(timestampMs);
  if (!isValid(date)) {
    return "暂无";
  }

  return format(date, "M月d日 HH:mm", { locale: zhCN });
}

export function formatIsoTimestamp(value: string | null | undefined) {
  if (!value) {
    return "暂无";
  }

  const date = new Date(value);
  if (!isValid(date)) {
    return "暂无";
  }

  return format(date, "yyyy-MM-dd HH:mm:ss");
}

export function formatIsoRelative(value: string | null | undefined) {
  if (!value) {
    return "暂无";
  }

  const date = new Date(value);
  if (!isValid(date)) {
    return "暂无";
  }

  return formatDistanceToNowStrict(date, { addSuffix: true, locale: zhCN });
}

export function formatPercent(value: number | null | undefined) {
  if (value === null || value === undefined) {
    return "暂无";
  }

  return `${value}%`;
}

export function toTitleCase(value: string) {
  return value
    .replace(/[-_]/g, " ")
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

export function fallbackText(value: string | null | undefined, fallback = "暂无") {
  return value && value.trim().length > 0 ? value : fallback;
}
