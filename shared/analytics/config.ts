// Public ingestion keys only. Never embed a personal/API-management credential.
export function browserAnalyticsDefinitions(env: Record<string, string | undefined>) {
  const requested = env.PUBLIC_POSTHOG_ENABLED === undefined || env.PUBLIC_POSTHOG_ENABLED === "true"
  const token = /^phc_[a-zA-Z0-9_]+$/.test(env.PUBLIC_POSTHOG_TOKEN || "") ? env.PUBLIC_POSTHOG_TOKEN! : ""
  const host = env.PUBLIC_POSTHOG_HOST ?? "https://eu.i.posthog.com"
  const environment = env.PUBLIC_POSTHOG_ENVIRONMENT ?? "production"
  const enabled = requested && !!token && host === "https://eu.i.posthog.com" && ["production", "synthetic", "development"].includes(environment)
  return {
    __APPARENT_ANALYTICS_ENABLED__: JSON.stringify(enabled),
    __APPARENT_ANALYTICS_CONFIG__: JSON.stringify({ enabled, token, host: enabled ? host : "", environment: enabled ? environment : "" }),
  }
}
