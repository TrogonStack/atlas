{{- define "trogon-atlas-server.name" -}}
{{- default "trogon-atlas-server" .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
  Standalone installs want a sane release-derived name. Umbrella installs
  (through the trogon-atlas chart's "server" alias) want the exact names an
  operator already depends on: release "test" renders "test-trogon-atlas-server",
  release "trogon-atlas" renders "trogon-atlas-server".

  1. fullnameOverride wins outright.
  2. A release name that already names this chart is used as-is.
  3. Otherwise anchor on "trogon-atlas": reuse the release name if it already
     carries that, else append it, then suffix "-server".
*/}}
{{- define "trogon-atlas-server.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else if contains "trogon-atlas-server" .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $base := .Release.Name }}
{{- if not (contains "trogon-atlas" .Release.Name) }}
{{- $base = printf "%s-trogon-atlas" .Release.Name }}
{{- end }}
{{- printf "%s-server" $base | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "trogon-atlas-server.chart" -}}
{{- printf "%s-%s" "trogon-atlas-server" .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "trogon-atlas-server.labels" -}}
helm.sh/chart: {{ include "trogon-atlas-server.chart" . }}
app.kubernetes.io/name: {{ include "trogon-atlas-server.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "trogon-atlas-server.selectorLabels" -}}
app.kubernetes.io/name: {{ include "trogon-atlas-server.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "trogon-atlas-server.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "trogon-atlas-server.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "trogon-atlas-server.image" -}}
{{- $registry := .root.Values.global.imageRegistry }}
{{- $tag := default .root.Chart.AppVersion .image.tag }}
{{- if $registry }}
{{- printf "%s/%s:%s" $registry .image.repository $tag }}
{{- else }}
{{- printf "%s:%s" .image.repository $tag }}
{{- end }}
{{- end }}

{{/* Name of the Secret holding the server tokens file, if any. */}}
{{- define "trogon-atlas-server.tokensSecretName" -}}
{{- with .Values.auth.tokensFile }}
{{- if .existingSecret }}
{{- .existingSecret }}
{{- else if .content }}
{{- printf "%s-auth" (include "trogon-atlas-server.fullname" $) }}
{{- end }}
{{- end }}
{{- end }}

{{/* Name of the Secret holding the shared server token, if any. */}}
{{- define "trogon-atlas-server.tokenSecretName" -}}
{{- with .Values.auth.token }}
{{- if .existingSecret }}
{{- .existingSecret }}
{{- else if .value }}
{{- printf "%s-auth" (include "trogon-atlas-server.fullname" $) }}
{{- end }}
{{- end }}
{{- end }}

{{/* Fail fast when the server has no usable authentication configuration. */}}
{{- define "trogon-atlas-server.validateAuth" -}}
{{- $auth := .Values.auth }}
{{- $hasTokensFile := or $auth.tokensFile.content $auth.tokensFile.existingSecret }}
{{- $hasToken := or $auth.token.value $auth.token.existingSecret }}
{{- if not (or $hasTokensFile $hasToken $auth.insecureAllowAnonymous) }}
{{- fail "server.auth: configure auth.tokensFile or auth.token, or set auth.insecureAllowAnonymous=true. The server refuses to start without authentication." }}
{{- end }}
{{- end }}

{{/*
  The write path is single-writer by construction. `mutation_lock` in
  service.rs is a process-local tokio Mutex, so cross-instance atomicity does
  not exist: two writer replicas can interleave a MergeBranch, and a second
  writer's mutations do not invalidate the first's in-process snapshot cache,
  so reads silently serve stale data until the next local mutation.

  Scaling the Deployment is therefore not a supported way to add write
  capacity, and the failure mode is silent. Refuse to render rather than let
  an operator reach it by accident.

  server.acknowledgeSingleWriter=true is the explicit override, for operators
  who have read the above and want extra replicas anyway (for example, an
  all-reader fleet behind a gateway that routes mutations elsewhere).
*/}}
{{- define "trogon-atlas-server.validateReplicas" -}}
{{- $server := .Values }}
{{- if and (gt (int $server.replicaCount) 1) (not $server.acknowledgeSingleWriter) }}
{{- fail (printf "server.replicaCount=%d: the trogon-atlas server is single-writer. Its mutation lock is process-local, so multiple replicas break BatchMutate/MergeBranch atomicity and serve stale reads from a stale per-process snapshot cache, with no error surfaced. Keep replicaCount=1, or set server.acknowledgeSingleWriter=true if every replica is read-only and mutations are routed elsewhere." (int $server.replicaCount)) }}
{{- end }}
{{- end }}

{{/*
  Authenticating the server while leaving the store open is a boundary the
  server cannot hold. Ownership lives in the namespace registry, which is a
  KV bucket like any other: anything that can reach NATS unauthenticated can
  rewrite which owner a namespace belongs to, or read every tenant's entities
  without going through an RPC at all. The server keeps enforcing, and the
  enforcement stops meaning anything.

  Refuse to render rather than let an operator reach that by accident, and
  offer the same explicit override as validateReplicas for the deployments
  where the credential genuinely lives elsewhere (a service mesh with mTLS,
  a NetworkPolicy that isolates the bucket).

  natsUrlSecret is opaque here, so a Secret is taken as credentialed.
*/}}
{{- define "trogon-atlas-server.validateStoreCredential" -}}
{{- $server := .Values }}
{{- $auth := $server.auth }}
{{- $hasTokensFile := or $auth.tokensFile.content $auth.tokensFile.existingSecret }}
{{- $hasToken := or $auth.token.value $auth.token.existingSecret }}
{{- if and (or $hasTokensFile $hasToken) (not $server.config.natsUrlSecret.name) (not $server.config.acknowledgeUnauthenticatedStore) }}
{{- $anonymous := list }}
{{- range (splitList "," $server.config.natsUrl) }}
{{- $url := trim . }}
{{- if and $url (not (regexMatch "^[a-z]+://[^/@]+@" $url)) }}
{{- $anonymous = append $anonymous $url }}
{{- end }}
{{- end }}
{{- if $anonymous }}
{{- fail (printf "server.config.natsUrl carries no credentials (%s) while server.auth is configured. The namespace registry that decides who owns what is a KV bucket in that store, so an unauthenticated NATS is a way around the server rather than a way into it. Put credentials in server.config.natsUrlSecret, or set server.config.acknowledgeUnauthenticatedStore=true if the store is closed some other way." (join ", " $anonymous)) }}
{{- end }}
{{- end }}
{{- end }}

{{/* Name of the Secret holding the SpiceDB preshared key, if any. */}}
{{- define "trogon-atlas-server.spicedbSecretName" -}}
{{- with .Values.spicedb.presharedKey }}
{{- if .existingSecret }}
{{- .existingSecret }}
{{- else if .value }}
{{- printf "%s-spicedb" (include "trogon-atlas-server.fullname" $) }}
{{- end }}
{{- end }}
{{- end }}

{{/*
  SpiceDB is all-or-nothing, and the half-configured cases are the dangerous
  ones.

  A key with no endpoint is somebody who believes SpiceDB is deciding
  visibility when the registry still is. The server refuses to start on that
  exact combination; the chart refuses to render it, so the answer arrives at
  `helm install` rather than as a CrashLoopBackOff.

  An endpoint with no key cannot connect at all, which is the same mistake
  pointing the other way.

  skipStartupSync is the third: startup sync and the token-file reload are
  one code path, so turning it off means no principal ever reaches SpiceDB
  from this process. Every check then answers no and every caller sees an
  empty model. Something else has to run `sync-spicedb`, and the CronJob in
  this chart is the only thing the chart can see, so require it.
*/}}
{{- define "trogon-atlas-server.validateSpicedb" -}}
{{- $spicedb := .Values.spicedb }}
{{- $hasKey := or $spicedb.presharedKey.value $spicedb.presharedKey.existingSecret }}
{{- if and $hasKey (not $spicedb.endpoint) }}
{{- fail "server.spicedb.presharedKey is set without server.spicedb.endpoint. Visibility is still decided by the namespace registry, not by SpiceDB. Set the endpoint, or clear the key." }}
{{- end }}
{{- if and $spicedb.endpoint (not $hasKey) }}
{{- fail "server.spicedb.endpoint is set without server.spicedb.presharedKey. Set presharedKey.value or presharedKey.existingSecret to the key SpiceDB was started with." }}
{{- end }}
{{- if and $spicedb.endpoint (not (has $spicedb.freshness (list "minimize-latency" "fully-consistent"))) }}
{{- fail (printf "server.spicedb.freshness=%q is not one of minimize-latency, fully-consistent." $spicedb.freshness) }}
{{- end }}
{{- if and $spicedb.skipStartupSync (not $spicedb.sync.enabled) }}
{{- fail "server.spicedb.skipStartupSync=true with server.spicedb.sync.enabled=false: nothing would ever publish grants to SpiceDB, so every permission check answers no and every caller sees an empty model. Enable the sync CronJob, or leave skipStartupSync off." }}
{{- end }}
{{- if and $spicedb.sync.enabled (not $spicedb.endpoint) }}
{{- fail "server.spicedb.sync.enabled=true without server.spicedb.endpoint: sync-spicedb has nothing to sync to and the Job would fail on every schedule." }}
{{- end }}
{{- end }}
