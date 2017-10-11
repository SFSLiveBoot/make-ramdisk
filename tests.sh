#!/bin/sh

: ${kver:=`uname -r`}
: ${arch:=`uname -m`}
: ${srv:=10.0.2.2}
: ${share:=live}
: ${http_srv:=http://$srv:8000}
: ${http_path:=$share}
: ${kernel:=/boot/vmlinuz-$kver}
: ${initrd:=ramdisk-$kver}
: ${initrd_net:=ramdisk_net-$kver}
: ${kvm_cmd:=kvm -m 4096 -vga qxl -snapshot}
: ${init:=/bin/systemd}
: ${test:=test_${1:-smb_direct}}
: ${dev:=/dev/sdb}

test -n "$srv_test" || {
  if test "x$srv" = "x10.0.2.2";then srv_test="127.0.0.1"
  else srv_test="$srv";fi
}

ERROR() {
  exit 1
}

test_http_copyup() {
  curl -s "$(echo "$http_srv/$http_path/" | sed -e 's@10\.0\.2\.2@127.0.0.1@')" | grep -q "\.sfs" || {
    echo "No sfs files found at $http_srv/$http_path/ " >&2
    return 1
  }
  $kvm_cmd -kernel "$kernel" -initrd "$initrd_net" -append "root=mem:$http_srv/$http_path/*.sfs+:$http_path/\$arch/*kernel-\$kver.sfs+mem ip=dhcp $append"
}

test_smb_direct() {
  smbclient //$srv_test/$share -c 'dir *.sfs' -U % | grep -q '\.sfs' || {
    echo "No .sfs files found on //$srv_test/$share SMB share"
    return 1
  }
  $kvm_cmd -kernel "$kernel" -initrd "$initrd_net" -append "root=smb://%@$srv/$share:./*.sfs+:./\$arch/*kernel-\$kver.sfs+mem ip=dhcp $append"
}

test_blockdev_uuid() {
  test -r "$dev" || ERROR "Cannot read '$dev'. Run: sudo setfacl -m u:$USER:r $dev"
  test -n "$uuid" || ERROR "Need UUID. export uuid=xxxxx before run."
  $kvm_cmd -drive file="$dev",if=virtio -kernel "$kernel" -initrd "$initrd" -append "root=UUID=$uuid:$share/*.sfs+:$share/\$arch/*kernel-\$kver.sfs+mem $append"
}

test_blockdev_uuid_mem() {
  test -r "$dev" || { echo "Need to be able to read \$dev ($dev)" >&2 ; return 1; }
  test -n "$uuid" || { echo "Need to define \$uuid ($uuid)" >&2 ; return 1; }
  $kvm_cmd -drive file="$dev",if=virtio,readonly -kernel "$kernel" -initrd "$initrd" -append "root=mem:UUID=$uuid:$share/*.sfs+:$share/\$arch/*kernel-\$kver.sfs+mem $append"
}

test_all() {
  for t in http_copyup smb_direct blockdev_uuid blockdev_uuid_mem;do
    set +x
    echo -n "Press enter to start test $t.."
    read x
    if { set -x ; test_$t; };then echo "Ok."; else echo "Failed.";fi
  done
}

set -x
make KVERS=$kver $initrd
make KVERS=$kver NET=1 $initrd_net
type "$test" && "$test"
