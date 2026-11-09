{{/*
The environment and the mounts of the server process, shared by the server
container and the definitions init container, which runs the same binary on
the same configuration (`ferroehr definitions load`).
*/}}
{{- define "ferroehr.serverEnv" -}}
# The server reads the file FERROEHR_CONFIG names and searches for
# none, so the mounted ferroehr.toml is named here.
- name: FERROEHR_CONFIG
  value: /etc/ferroehr/ferroehr.toml
{{- if .Values.auth.caExistingSecret }}
# The issuer's CA, beside the image's own roots (`[auth] ca_file`).
- name: FERROEHR__AUTH__CA_FILE
  value: /etc/ferroehr-issuer-ca/ca.crt
{{- end }}
{{- if or .Values.database.existingSecret .Values.database.url }}
{{- /* The DSN is the credential that reaches patient data, so it is a
       mounted file and only its PATH is environment: an env VALUE is
       readable through /proc/<pid>/environ and inherited by every
       child process. `DATABASE_URL` is deliberately NOT set — the
       server recognises it as a lower-priority alias, and setting both
       it and a file path would work but read ambiguously. */}}
- name: FERROEHR__DATABASE__URL_FILE
  value: {{ printf "%s/db.url" (include "ferroehr.secretMountPath" .) | quote }}
{{- end }}
{{- range $domain := list "clinical" "party" "audit" }}
{{- $dsn := index $.Values.database $domain }}
{{- if or $dsn.existingSecret $dsn.url }}
{{- /* One storage domain's own credential, mounted the same way and
       for the same reason as the shared DSN. Set, it makes the
       schema separation a credential separation too, and lets the
       domain live in a database of its own: one leaked DSN then
       reaches one domain rather than all of them. Unset, the domain
       shares the DSN above and the separation is schema-only, which
       the boot self-check still enforces. */}}
- name: FERROEHR__DATABASE__{{ upper $domain }}__URL_FILE
  value: {{ printf "%s/storage.%s.url" (include "ferroehr.secretMountPath" $) $domain | quote }}
{{- end }}
{{- if $dsn.migrateExistingSecret }}
{{- /* The migration credential of the database this domain lives in,
       when that is not the database the shared migration DSN
       reaches: preparation runs per database, and under
       migrate=verify it reads that database's ext and domain
       bookkeeping, which the domain's DML-only role cannot. */}}
- name: FERROEHR__DATABASE__{{ upper $domain }}__MIGRATE_URL_FILE
  value: {{ printf "%s/storage.migrate.%s.url" (include "ferroehr.secretMountPath" $) $domain | quote }}
{{- end }}
{{- end }}
{{- if or .Values.database.migrateExistingSecret .Values.database.migrateUrl }}
{{- /* The credential schema preparation runs as, mounted the same
       way and for the same reason. Preparation spans every schema —
       each resident migration set's DDL under migrate=apply, their
       bookkeeping tables under verify — so a pod whose runtime DSNs
       are domain-scoped roles cannot do it on either of them. Unset,
       preparation falls back to the clinical DSN. */}}
- name: FERROEHR__DATABASE__MIGRATE_URL_FILE
  value: {{ printf "%s/db.migrate_url" (include "ferroehr.secretMountPath" .) | quote }}
{{- end }}
{{- if .Values.secrets.signingKeyPassphrase }}
- name: FERROEHR__RECORD__SIGNING__KEY_PASSPHRASE_FILE
  value: {{ printf "%s/signing.key_passphrase" (include "ferroehr.secretMountPath" .) | quote }}
{{- end }}
{{- if .Values.secrets.auditFhirFeedUrl }}
- name: FERROEHR__AUDIT__FHIR_FEED__URL
  valueFrom:
    secretKeyRef:
      name: {{ include "ferroehr.secretName" . }}
      key: FERROEHR__AUDIT__FHIR_FEED__URL
{{- end }}
{{- if .Values.secrets.multimediaAccessKeyId }}
- name: FERROEHR__RECORD__MULTIMEDIA__ACCESS_KEY_ID
  valueFrom:
    secretKeyRef:
      name: {{ include "ferroehr.secretName" . }}
      key: FERROEHR__RECORD__MULTIMEDIA__ACCESS_KEY_ID
{{- end }}
{{- if .Values.secrets.multimediaSecretAccessKey }}
- name: FERROEHR__RECORD__MULTIMEDIA__SECRET_ACCESS_KEY_FILE
  value: {{ printf "%s/multimedia.secret_access_key" (include "ferroehr.secretMountPath" .) | quote }}
{{- end }}
{{- with .Values.extraEnv }}
{{- toYaml . | nindent 0 }}
{{- end }}
{{- end }}

{{- define "ferroehr.serverVolumeMounts" -}}
# readOnlyRootFilesystem=true → give the process a writable scratch dir.
- name: tmp
  mountPath: /tmp
# ferroehr.toml (+ any config.files) mounted read-only at /etc/ferroehr.
- name: config
  mountPath: /etc/ferroehr
  readOnly: true
{{- if eq (include "ferroehr.hasFileSecrets" .) "true" }}
# File-borne secrets, read-only. NOT a subPath mount: a subPath-mounted
# Secret never receives updates, so a rotation would not propagate
# (https://kubernetes.io/docs/concepts/storage/volumes/#using-subpath).
- name: secrets
  mountPath: {{ include "ferroehr.secretMountPath" . }}
  readOnly: true
{{- end }}
{{- if .Values.database.caConfigMap }}
{{- include "ferroehr.dbCaMount" . | nindent 0 }}
{{- end }}
{{- if .Values.auth.caExistingSecret }}
- name: issuer-ca
  mountPath: /etc/ferroehr-issuer-ca
  readOnly: true
{{- end }}
{{- with .Values.extraVolumeMounts }}
{{- toYaml . | nindent 0 }}
{{- end }}
{{- end }}
