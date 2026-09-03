#!/bin/sh
# Regenerate one release cohort's descriptors inside a service image build.
#
# The image has no Git checkout, so it cannot decide which revision it is: the
# revision is handed in and this proves it. `image-descriptors` rehashes the
# service executables this build just produced, rehashes every shipped contract
# schema, and refuses to emit anything unless the regenerated source and owner
# descriptors are byte-identical to the ones the checkout ships and the schema
# digests match the checked-in manifest. Only the service and frontend digests
# may differ from the owner cohort, because those are the bytes this build made.
#
# The result is the manifest that ships inside the image: a consumer reads it
# and the image labels and needs no owner checkout to verify either.
#
# usage: release-inside-image.sh <checkout-root> <output-dir> <source-revision>
set -eu

root=$1
output=$2
revision=$3

exec python3 "$root/tools/scripts/release_qualification.py" image-descriptors \
    --root "$root" \
    --source-revision "$revision" \
    --compilation-service target/release/apxm-compilation-service \
    --runtime-service target/release/apxm-runtime-service \
    --python-frontend-native crates/compiler/frontend/python/apxm_program/_native.so \
    --output-dir "$output"
