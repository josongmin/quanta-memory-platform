#!/usr/bin/env bash
# Opt-in L2 block-device disruption rail. This uses only a newly-created sparse
# image, loop device, and uniquely-named device-mapper mapping. It is not L3.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

if [[ "${QMP_DURABILITY_L2_DM_ALLOW_DISPOSABLE:-}" != "1" ]]; then
  echo "UNAVAILABLE: set QMP_DURABILITY_L2_DM_ALLOW_DISPOSABLE=1 only inside a disposable Linux VM" >&2
  exit 77
fi
if [[ "$(uname -s)" != Linux || "${EUID}" != 0 ]]; then
  echo "UNAVAILABLE: device-mapper L2 requires Linux and effective UID 0" >&2
  exit 77
fi
for tool in cargo python3 strace timeout kill dmsetup losetup blockdev mkfs.ext4 mount umount mountpoint findmnt e2fsck realpath sha256sum; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "UNAVAILABLE: required tool missing: ${tool}" >&2
    exit 77
  fi
done
if ! dmsetup targets | awk '$1 == "error" { found=1 } END { exit !found }'; then
  echo "UNAVAILABLE: kernel device-mapper error target is unavailable" >&2
  exit 77
fi

artifact_root_input="${QMP_DURABILITY_L2_ARTIFACT_ROOT:-${TMPDIR:-/tmp}/qmp-durability-dm-$$}"
artifact_root="$(realpath -m -- "${artifact_root_input}")"
user_home="${HOME:-/nonexistent}"
case "${artifact_root}" in
  /|"${repo_root}"|"${repo_root}"/*|"${user_home}"|"${user_home}"/*)
    echo "error: artifacts must use a new narrow path outside repository and home" >&2
    exit 2
    ;;
esac
if [[ -e "${artifact_root}" ]]; then
  echo "error: artifact directory already exists: ${artifact_root}" >&2
  exit 2
fi

vm_root="$(mktemp -d /tmp/qmp-durability-dm.XXXXXX)"
image="${vm_root}/backing.img"
mount_path="${vm_root}/mount"
dm_suffix="$(tr -d '-' </proc/sys/kernel/random/uuid)"
dm_name="qmpd${dm_suffix}"
loop_device=""
loop_used=""
mapping_created=false
mounted=false
# shellcheck disable=SC2317,SC2329 # Called indirectly by EXIT trap; 0.9 and 0.11 report different codes.
cleanup() {
  local status=$?
  trap - EXIT
  # Cleanup must continue when evidence cannot be persisted (for example, a
  # caller supplied a read-only artifact mount). Do not let the script's
  # fail-fast mode skip device detach or the runner-owned temp-directory drop.
  set +e
  if [[ "${mapping_created}" == true && -n "${loop_device}" ]] && dmsetup info "${dm_name}" >/dev/null 2>&1; then
    dmsetup suspend --noflush "${dm_name}" >/dev/null 2>&1 || true
    dmsetup reload "${dm_name}" --table "0 ${sectors} linear ${loop_device} 0" >/dev/null 2>&1 || true
    dmsetup resume "${dm_name}" >/dev/null 2>&1 || true
  fi
  if [[ "${mounted}" == true ]]; then
    umount "${mount_path}" || status=1
  fi
  if [[ "${mapping_created}" == true ]] && dmsetup info "${dm_name}" >/dev/null 2>&1; then
    dmsetup remove "${dm_name}" || status=1
  fi
  if [[ -n "${loop_device}" ]]; then
    losetup -d "${loop_device}" || status=1
  fi
  if [[ -n "${artifact_root}" && -f "${image}" ]]; then
    if mkdir -p "${artifact_root}" && \
      cp --sparse=always "${image}" "${artifact_root}/device-mapper-backing.img"; then
      local image_hash image_bytes
      image_hash="$(sha256sum "${artifact_root}/device-mapper-backing.img" | awk '{print $1}')"
      image_bytes="$(stat -c '%s' "${artifact_root}/device-mapper-backing.img")"
      if [[ -n "${image_hash}" && -n "${image_bytes}" ]]; then
        if [[ -f "${artifact_root}/receipt.txt" ]]; then
          printf 'device_mapper_image_sha256=%s\ndevice_mapper_image_bytes=%s\n' \
            "${image_hash}" "${image_bytes}" >>"${artifact_root}/receipt.txt" || status=1
        else
          printf 'level=L2\nrunner_receipt=unavailable\nrunner_exit_status=%s\ndevice_mapper_image_sha256=%s\ndevice_mapper_image_bytes=%s\n' \
            "${status}" "${image_hash}" "${image_bytes}" >"${artifact_root}/receipt.txt" || status=1
        fi
      else
        echo "error: copied DM backing image could not be hashed/stat'ed" >&2
        status=1
      fi
    else
      echo "error: could not preserve DM backing image under ${artifact_root}" >&2
      status=1
    fi
  fi
  rm -rf -- "${vm_root}" || status=1
  exit "${status}"
}
trap cleanup EXIT

mkdir -p "${mount_path}"
truncate -s 128M "${image}"
loop_device="$(losetup --find --show "${image}")"
loop_used="${loop_device}"
sectors="$(blockdev --getsz "${loop_device}")"
dmsetup create "${dm_name}" --table "0 ${sectors} linear ${loop_device} 0"
mapping_created=true
dmsetup mknodes "${dm_name}"
mkfs.ext4 -F -q "/dev/mapper/${dm_name}"
mount -o noatime "/dev/mapper/${dm_name}" "${mount_path}"
mounted=true

export QMP_DURABILITY_L2_ARTIFACT_ROOT="${artifact_root}"
export QMP_DURABILITY_L2_STORAGE_ROOT="${mount_path}"
export QMP_DURABILITY_L2_DM_NAME="${dm_name}"
export QMP_DURABILITY_L2_DM_LOOP="${loop_device}"
export QMP_DURABILITY_L2_DM_SECTORS="${sectors}"
export QMP_DURABILITY_L2_DM_MOUNT="${mount_path}"
export QMP_DURABILITY_L2_MODE=device-mapper

echo "L2 mechanism=dm-error-table-swap dm=${dm_name} backing=${loop_device} bytes=134217728 claim=block-device EIO recovery only"
set +e
scripts/durability/run-emulated-linux.sh
runner_status=$?
set -e
if ! mkdir -p "${artifact_root}"; then
  echo "error: could not create artifact directory after L2 runner: ${artifact_root}" >&2
  runner_status=1
fi
if [[ ! -f "${artifact_root}/receipt.txt" && -d "${artifact_root}" ]]; then
  printf 'level=L2\nrunner_receipt=unavailable\nrunner_exit_status=%s\n' \
    "${runner_status}" >"${artifact_root}/receipt.txt" || runner_status=1
fi
if [[ "${mounted}" == true ]]; then
  if umount "${mount_path}"; then
    mounted=false
  else
    runner_status=1
  fi
fi
if [[ "${mapping_created}" == true ]]; then
  if dmsetup info "${dm_name}" >/dev/null 2>&1; then
    if dmsetup remove "${dm_name}"; then
      mapping_created=false
    else
      runner_status=1
    fi
  else
    mapping_created=false
  fi
fi
if losetup -d "${loop_device}"; then
  loop_device=""
else
  runner_status=1
fi
printf 'device_mapper_name=%s\ndevice_mapper_loop=%s\ndevice_mapper_sectors=%s\n' \
  "${dm_name}" "${loop_used}" "${sectors}" >>"${artifact_root}/receipt.txt" || runner_status=1
exit "${runner_status}"
