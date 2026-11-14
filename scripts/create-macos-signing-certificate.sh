#!/usr/bin/env bash
# Create ONE long-lived self-signed code-signing certificate for Mac releases.
# It gives every build the same macOS identity, so Screen Recording and
# Accessibility grants survive updates. It is not Developer ID: Gatekeeper
# still warns on first open and the build is not notarized.
#
# Run once, on a trusted Mac. Replacing the certificate later forces every
# user to grant the permissions again, so back up the output directory.
set -euo pipefail
output_dir="${1:-$HOME/sanser-macos-signing}"
if [[ "$output_dir" == "$(cd "$(dirname "$0")/.." && pwd)"* ]]; then
  echo 'Choose an output directory outside the repository.' >&2; exit 2
fi
if [[ -e "$output_dir/sanser-signing.p12" ]]; then
  echo "A certificate already exists in $output_dir; refusing to replace it." >&2; exit 2
fi
umask 077
mkdir -p "$output_dir"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
# The system LibreSSL writes a PKCS#12 that `security import` accepts.
openssl=/usr/bin/openssl
cat > "$work/certificate.cnf" <<'EOF'
[req]
distinguished_name=dn
x509_extensions=v3
prompt=no
[dn]
CN=Sanser Self-Signed Release
[v3]
basicConstraints=critical,CA:false
keyUsage=critical,digitalSignature
extendedKeyUsage=critical,codeSigning
EOF
password="$("$openssl" rand -hex 24)"
"$openssl" req -x509 -newkey rsa:3072 -nodes -days 7300 -config "$work/certificate.cnf" \
  -keyout "$work/key.pem" -out "$work/certificate.pem" 2>/dev/null
"$openssl" pkcs12 -export -inkey "$work/key.pem" -in "$work/certificate.pem" \
  -name 'Sanser Self-Signed Release' -out "$output_dir/sanser-signing.p12" -passout "pass:$password"
printf '%s' "$password" > "$output_dir/sanser-signing.password"
base64 < "$output_dir/sanser-signing.p12" | tr -d '\n' > "$output_dir/sanser-signing.p12.base64"
fingerprint="$("$openssl" x509 -in "$work/certificate.pem" -noout -fingerprint -sha1 | cut -d= -f2 | tr -d :)"
cat <<EOF
Created in $output_dir (keep it private and backed up):
  sanser-signing.p12          certificate and private key
  sanser-signing.password     its password
  sanser-signing.p12.base64   the value for the GitHub secret

Certificate SHA-1: $fingerprint

Add two GitHub Actions repository secrets:
  MACOS_SELF_SIGNED_CERTIFICATE           contents of sanser-signing.p12.base64
  MACOS_SELF_SIGNED_CERTIFICATE_PASSWORD  contents of sanser-signing.password
EOF
