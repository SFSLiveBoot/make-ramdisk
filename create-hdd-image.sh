#!/bin/sh

hdd_image="$1"
first_file="$2"

: ${image_size:=8G}
: ${part_offset:=0}

test "$part_offset" -gt 0 || part_offset=""

export PATH="$PATH:/sbin:/usr/sbin"

test -r "$first_file" -a -n "$hdd_image" -a ! -e "$hdd_image" || {
  echo "Usage: ${0##*/} <new_image_file> <src_files..>" 2>&1
  exit 1
}

shift

gen_debugfs_cp() {
  local bn subdir src_file src_loc="$src_loc"
  test -n "$src_loc" || src_loc="${1%/*}"

  for src_file;do
    bn="${src_file##*/}"
    subdir="${src_file%/$bn}"
    subdir="${subdir#$src_loc}"
    test -z "$subdir" || ( IFS=/
      for sd in $subdir;do
        test -n "$sd" || continue
        echo mkdir "$sd"
        echo cd "$sd"
      done)
    echo write "$src_file" "$bn"
    test -z "$subdir" || ( IFS=/
      for sd in $subdir;do
        test -n "$sd" || continue
        echo cd ..
      done
    )
  done
}

qemu-img create "$hdd_image" "$image_size"
mkfs.ext4 ${part_offset:+-E offset=$(($part_offset*512))} "$hdd_image"

test -z "$part_offset" || {
  parted "$hdd_image" mklabel msdos
  parted "$hdd_image" mkpart primary ext2 ${part_offset}s 100%
  parted "$hdd_image" toggle 1 boot
}

: ${dist:=$(basename "$(dirname "$first_file")")}

debugfs -w "$hdd_image"${part_offset:+?offset=$(($part_offset*512))} <<EOF
mkdir $dist
cd $dist
$(gen_debugfs_cp "$@")
q
EOF
