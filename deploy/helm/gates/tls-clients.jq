# The facts that decide how the chart's own clients reach the CDR listener, one
# "<fact> <value>" line each, over a rendered manifest slurped into an array.
# Read by validate.sh `server_tls_gate`, which compares them with what the
# server's own TLS setting (config.server.tls.enabled) requires. $release is
# the CDR's fullname; the viewer and FerroBRIDGE workloads are named after it.
def deployment($name): .[] | select(.kind == "Deployment" and .metadata.name == $name);
def container_of($name): deployment($name) | .spec.template.spec.containers[0];

(container_of($release + "-viewer") | .env[]? | select(.name == "FERROEHR_VIEWER__CDR__BASE_URL") | "viewer-url " + .value),
(.[] | select(.kind == "ConfigMap" and .metadata.name == ($release + "-bridge")) | .data["config.toml"]
  | capture("\\[cdr\\]\\n(?:[ \\t]+[a-z_]+ = [^\\n]*\\n)*?[ \\t]+base_url = \"(?<url>[^\"]+)\"") | "bridge-url " + .url),
(("viewer", "bridge") as $w
  | deployment($release + "-" + $w) | .spec.template.spec
  | ((.containers[0].env[]? | select(.name == "SSL_CERT_DIR") | $w + "-certdir " + .value),
     (.volumes[]? | select(.name == "cdr-ca") | $w + "-ca-secret " + .secret.secretName),
     (.containers[0].volumeMounts[]? | select(.name == "cdr-ca") | $w + "-ca-mount " + .mountPath))),
(container_of($release) | (.livenessProbe, .readinessProbe, .startupProbe) | select(. != null) | "probe-scheme " + (.httpGet.scheme // "HTTP")),
(.[] | select(.kind == "Service" and .metadata.name == $release) | .spec.ports[] | select(.name == "http") | "service-appprotocol " + .appProtocol),
(.[] | select(.kind == "ServiceMonitor") | .spec.endpoints[0]
  | ("scrape-scheme " + .scheme), ("scrape-ca-secret " + (.tlsConfig.ca.secret.name // ""))),
(deployment($release) | .spec.template.metadata.annotations // {}
  | select(.["prometheus.io/scrape"] == "true")
  | "annotation-scheme " + (.["prometheus.io/scheme"] // "http")),
(.[] | select(.kind == "Ingress" and .metadata.name == $release)
  | "ingress-backend-protocol " + ((.metadata.annotations // {})["nginx.ingress.kubernetes.io/backend-protocol"] // "absent"))
