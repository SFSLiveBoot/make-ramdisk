KVERS=$(shell uname -r)
ARCH_LIB=$(dir $(shell ldd /bin/sh | grep "libc.so." | grep -o '/lib[^[:space:]]*'))
MODDIR=/lib/modules/$(KVERS)/

SHELL=/bin/bash
PATH:=$(PATH):/sbin:/usr/sbin
RAMDISK=ramdisk$(RAMDISK_EXTRAS)-$(KVERS)
ifndef RD_DIR
  RD_DIR:=$(shell mktemp -d /tmp/ramdisk.XXXXXX)
  export RD_DIR
endif

RD_TMPL=ramdisk.template
RD_FILES=$(shell find $(RD_TMPL) -not -type l)

findprog=$(shell find {/usr,}/{s,}bin -maxdepth 1 \( -false $(foreach name,$(1),-or -name "$(name)") \))
findmod=$(basename $(notdir $(shell find $(MODDIR) \( -false $(foreach name,$(1),-or -name "$(subst -,[-_],$(name)).ko") \))))

ISO=cdrom.iso
ISO_LABEL=Linux Live
ISO_DIR=$(patsubst %.iso,%.d,$(ISO))
ISO_TMPL=cdrom.template
ISO_FILES=$(shell find $(ISO_TMPL) -not -type l)

SFS_PARTS=base kernel mods
SFS_DIR=netinstall/nfs

LAST_FS=64M

SFS_EXCLUDE=root/.ssh/authorized_keys etc/ssh/ssh_host_dsa_key etc/ssh/ssh_host_dsa_key.pub

MKISOFS=genisoimage -r -jcharset UTF-8


MODS_RUNNING=$(shell grep -v '^[^ ]* [^ ]* 0 ' /proc/modules | cut -f1 -d" " | grep -vE "^(snd|xt|nf|ipt|iptable|ip6|ip6t|ip6table)_")

NET_DISABLED_WIFI=wireless/ wlan
NET_DISABLED_PPP=ppp
NET_DISABLED=arcnet/ phy/ appletalk/ tokenring/ wan/ pcmcia/ hamradio/ irda/ $(NET_DISABLED_PPP) $(NET_DISABLED_WIFI)

NIC_DRV=$(basename $(notdir $(shell find $(MODDIR)/kernel/drivers/net -name '*.ko' $(patsubst %,-not -path '*/%*',$(NET_DISABLED)))))
NETDRV=$(NIC_DRV) $(call findmod,cifs nfs md4 hmac des_generic ecb arc4 nbd)

USBHID_MODS=$(call findmod,uhci-hcd ehci-hcd ohci-hcd usbhid hid-generic)
USBMODS=$(USBHID_MODS) $(call findmod,sd-mod usb-storage) $(notdir $(shell find $(MODDIR)/kernel/drivers/usb/{storage,host} -name "*.ko"))
DISKDRV=$(basename $(notdir $(shell find $(MODDIR)/kernel/drivers \( -path "$(MODDIR)/kernel/drivers/ata/*" -o -path "$(MODDIR)/kernel/drivers/scsi/*" -o -path "$(MODDIR)/kernel/drivers/ide/*" \) -name '*.ko'))) $(call findmod,cciss mptspi mptsas mmc_block sdhci_pci virtio_blk virtio_pci loop)

CRYPTOMODS=dm-crypt $(basename $(notdir $(shell find $(MODDIR)/kernel -name cbc.ko $(patsubst %,-or -name '%*.ko',aes sha arc4 xts))))
CRYPTOPROGS=cryptsetup

FSMODS=$(call findmod,ext2 ext3 ext4 xfs ntfs vfat reiserfs isofs)
OTHERMODS=aufs squashfs

FUSEPROGS=/sbin/mount.fuse
FUSEMODS=fuse

NTFSPROGS=$(call findprog,ntfs* scrounge-ntfs)

UDEVPROGS=udev{d,adm} $(wildcard /lib/libnss_files.so.* $(ARCH_LIB)/libnss_files.so.*) /sbin/{dmsetup,blkid} $(shell find $$(dpkg -L udev  | grep '^/lib/udev/[^/]*$$') -maxdepth 0 -type f)
UDEVFILES=$(shell dpkg -L udev dmsetup | grep -E '^/lib/udev/rules.d/') $(shell find /lib/udev/keymaps -type f)

NETPROGS=$(wildcard /lib/libnss_dns.so.* /lib/libnss_files.so.* $(ARCH_LIB)/libnss_files.so.* $(ARCH_LIB)/libnss_dns.so.*) $(call findprog,telnet udp-[rs]e*er *mount.cifs socat xnbd-client)

WIFIMODS=$(basename $(notdir $(shell find $(MODDIR)/kernel/drivers/net/wireless -name '*.ko')))
WIFIPROGS=iwlist iwconfig

DISKMODS=$(DISKDRV) $(call findmod,nls-cp437 nls-iso8859-1 nls-utf8 cdrom i2c-i801)
MODS=$(DISKMODS) $(FSMODS) $(OTHERMODS) $(USBMODS)
NORMMODS=yenta_socket $(CRYPTOMODS)

MINPROGS=modprobe /usr/lib/klibc/bin/* $(UDEVPROGS)
NORMPROGS=rmmod halt busybox losetup $(call findprog,fdisk lspci lvm kexec) $(CRYPTOPROGS)

TPM_PROGS=tcsd
TPM_MODS=$(basename $(shell find $(MODDIR) -name "tpm*.ko"))

PKCS11_PROGS=$(call findprog,openssl pcscd pkcs15-tool ifdhandler openct-control opensc-tool) /usr/lib/opensc-pkcs11.so /usr/lib/ssl/engines/engine_pkcs11.so $(shell find /usr/lib/pcsc/drivers -name "*.so") $(shell awk '/^LIBPATH/{print $$2}' /etc/reader.conf.d/*)
PKCS11_FILES=/etc/opensc/opensc.conf  /etc/openct.conf $(wildcard /etc/reader.conf.d/*) $(shell find /usr/lib/pcsc/drivers -type f -not -name "*.so")

PKCS15_PROGS=$(call findprog,pkcs15-tool pcscd) $(shell find /usr/lib/pcsc/drivers -name "*.so*") $(shell /sbin/ldconfig -p | grep -o '/[^[:space:]]*/libpcsclite.so.1' | head -1) $(shell /sbin/ldconfig -p | grep -o '/[^[:space:]]*/libgcc_s.so.1' | head -1)
PKCS15_FILES=$(shell find /usr/lib/pcsc/drivers -not -name "*.so*" -not -type d)

MODFILES=$(shell find "$(MODDIR)" -name "modules.builtin" -o -name "modules.order")
9P_MODS=9p $(call findmod,9pnet_virtio virtio_pci virtio_mmio virtio_balloon virtio_net)

RELAXMODS=fuse cdrom ehci-hcd loop ohci-hcd uhci-hcd aufs ext2 ext3 ext4 sd-mod yenta_socket reiserfs sdhci_pci

DATAFILES=$(MODFILES) $(UDEVFILES)

ifeq ($(MODS),dep)
MODS=$(MODS_RUNNING)
endif

ifeq ($(TGT),vmware)
NIC_DRV=pcnet32
endif

ifeq ($(TGT),usb)
DISKDRV=
FSMODS=isofs squashfs vfat
endif

ifeq ($(TGT),nfsroot)
FSMODS=
DISKMODS=
UDEVPROGS=
UDEVFILES=
NETPROGS=
NETDRV=$(NIC_DRV) nfs
USBMODS=$(USBHID_MODS)
MIN=1
NET=1
RAMDISK_EXTRAS?=_nfs
endif

ifdef SSH_PUBKEY
DROPBEAR=1
endif

ifdef DROPBEAR
NET=1
PROGS+=dropbear
endif

ifdef NTFS
PROGS+=$(NTFSPROGS) 
FUSE=1
endif

ifdef FUSE
MODS+=$(FUSEMODS) 
PROGS+=$(FUSEPROGS)
endif

ifdef 9P
MODS+=$(9P_MODS)
endif

ifdef WIFI
PROGS+=wpa_supplicant
NET_DISABLED_WIFI=
NET=1
RAMDISK_EXTRAS?=_wifi
endif

ifdef NET
PROGS+=$(NETPROGS)
MODS+=$(NETDRV) 
FSMODS+=nfs
RAMDISK_EXTRAS?=_net
endif

ifdef TPM
PROGS+=$(TPM_PROGS)
MODS+=$(TPM_MODS)
endif

ifdef PKCS11
PROGS+=$(PKCS11_PROGS)
DATAFILES+=$(PKCS11_FILES)
endif

ifdef PKCS15
PROGS+=$(PKCS15_PROGS)
DATAFILES+=$(PKCS15_FILES)
endif

PROGS+=$(MINPROGS)

ifdef MODS_PRELOAD
MODS+=$(MODS_PRELOAD)
endif

ifdef USWSUSP
PROGS+=/usr/lib/uswsusp/resume
DATAFILES+=/etc/uswsusp.conf
endif

ifndef MIN
PROGS+=$(NORMPROGS)
MODS+=$(NORMMODS)
DATAFILES+=/lib/terminfo/l/linux /usr/share/misc/pci.ids
endif

ifdef INST
PROGS+=cdebootstrap mkfs.ext4
DATAFILES+=/usr/share/keyrings/debian-archive-keyring.gpg
DATADIRS+=/usr/share/cdebootstrap
endif

ifdef EXTRA
EXTRAPROGS+=halt /sbin/{fsck,mkfs.}* $(call findprog,cfdisk hexedit less strace xfs_repair partimage grub hd parted cdebootstrap testdisk photorec ms-sys wlanconfig mksquashfs) lsmod pcimodules gzip objdump $(NTFSPROGS) ldconfig mount $(NETPROGS)
EXTRAMODS+=$(NETDRV)
DATADIRS+=/usr/lib/grub /usr/share/cdebootstrap
endif

DATAFILES+=$(EXTRA_DATAFILES)
PROGS+=$(EXTRAPROGS)
MODS+=$(EXTRAMODS)

KVM_APPEND=root=none quiet console=ttyS0
KVM_OPTS=-nographic


# some help variables to get around makefile syntax
empty=
space=$(empty) $(empty)
comma=,

# function list starts at word 57
BBHELP=$(shell busybox --help)
BBFUNC=$(subst $(comma),,$(wordlist 57,$(words $(BBHELP)),$(BBHELP)))

.PHONY:	all ramdisk clean test nfsroot dbg

all:	ramdisk

ramdisk:	$(RAMDISK)
	$(RM) -r $(RD_DIR)

clean:
	$(RM) -r $(RD_DIR) $(RAMDISK) $(ISO)

mrproper: clean
	$(RM) -r $(ISO_DIR)

test:	$(RAMDISK)
	$(RM) -r $(RD_DIR)
	kvm -kernel /boot/vmlinuz-$(KVERS) -initrd $(RAMDISK) -append "$(KVM_APPEND)" $(KVM_OPTS)

nfsroot:	clean
	$(MAKE) $(MAKEFLAGS) TGT=nfsroot
	cp $(RAMDISK) netinstall/tftp

$(RAMDISK): $(RD_FILES) Makefile
	cp -T -r $(RD_TMPL) $(RD_DIR)
	env LANG=C ./copy_exe -m $(RD_DIR) $(PROGS)
	test -z "$(DATADIRS)" || cp --parents -r $(DATADIRS) $(RD_DIR)
	test -z "$(DATAFILES)" || cp --parents $(DATAFILES) $(RD_DIR)
	test ! -e "$(RD_DIR)/sbin/udevadm" -o -e "$(RD_DIR)/sbin/udevsettle" || ln -s udevadm "$(RD_DIR)/sbin/udevsettle"
	test ! -e "$(RD_DIR)/usr/lib/klibc/bin/sh.shared" -o -e "$(RD_DIR)/bin/sh" || ln -s ../usr/lib/klibc/bin/sh.shared "$(RD_DIR)/bin/sh"
	test ! -e "$(RD_DIR)/bin/sh" -o -e "$(RD_DIR)/bin/bash" || ln -s sh "$(RD_DIR)/bin/bash"
	test -z "$(MODS_PRELOAD)" || for mod in $(MODS_PRELOAD);do echo "$$mod" ; done > "$(RD_DIR)/etc/modules.preload"
	test -z "$(DROPBEAR)" || { mkdir -p $(RD_DIR)/etc/dropbear $(RD_DIR)/scripts/rootfs.d; for t in rsa dss;do dropbearkey -t $$t -f $(RD_DIR)/etc/dropbear/dropbear_$${t}_host_key;done; echo "mkdir -p /dev/pts;mount -t devpts none /dev/pts || true; chmod 755 /; dropbear -E -s" >$(RD_DIR)/scripts/rootfs.d/dropbear.sh; }
	test -z "$(SSH_PUBKEY)" || { mkdir -p $(RD_DIR)/.ssh ; echo "$(SSH_PUBKEY)" >$(RD_DIR)/.ssh/authorized_keys ; }
	test -z "$(APPEND)" || echo "$(APPEND)" >$(RD_DIR)/cmdline
	grep -h -o "GROUP=[^ ]*" "$(RD_DIR)/lib/udev/rules.d"/*.rules | sed -e 's/GROUP="\([^"]*\)".*/^\1:/' | sort -u | grep -f - /etc/group | cut -f1-3 -d: | sed -e 's/$$/:/' >"$(RD_DIR)/etc/group"
	env MODDIR=$(MODDIR) KVERS=$(KVERS) ./moddep $(RD_DIR) -r "$(RELAXMODS)" $(sort $(basename $(notdir $(MODS))))
	test -z "$(NO_MAKEFLAGS)" || echo "$$MAKEFLAGS" >"$(RD_DIR)/.makeflags"
	(cd "$(RD_DIR)"; find . -not -name . | fakeroot cpio -L -V -o -H newc) | gzip -1 >"$(RAMDISK)"

$(ISO_DIR):	$(ISO_FILES) ramdisk
	cp -T -r $(ISO_TMPL) $(ISO_DIR)
	ln -sf ../../$(RAMDISK) $(ISO_DIR)/boot/ramdisk
	ln -sf /boot/vmlinuz-$(KVERS) $(ISO_DIR)/boot/vmlinuz
	sed -i -e 's|@iso_label@|$(ISO_LABEL)|g' -e 's|@root_dev@|/vol/$(subst $(space),_,$(ISO_LABEL))|g' -e 's|@last_fs@|$(LAST_FS)|g' -e 's|@fs_parts@|$(subst $(space),+,$(addsuffix .sfs,$(addprefix :boot/,$(SFS_PARTS))))|g' $(ISO_DIR)/boot/grub/menu.lst
	$(RM) $(ISO_DIR)/boot/grub/menu.lste
	sed -i -e 's|@iso_label@|$(ISO_LABEL)|g' $(ISO_DIR)/vmware/livecd.vmx
	$(RM) $(ISO_DIR)/vmware/livecd.vmxe
	@touch $@

$(ISO):	$(ISO_DIR) $(addprefix $(ISO_DIR)/boot/,$(addsuffix .sfs,$(SFS_PARTS))) Makefile
	$(MKISOFS) -f -c boot/boot.catalog -b boot/grub/stage2_eltorito -no-emul-boot -boot-load-size 4 -boot-info-table -V "$(ISO_LABEL)" -o $(ISO) $(ISO_DIR)

$(ISO_DIR)/boot/%.sfs:	$(SFS_DIR)/%/
	fakeroot $(patsubst %,-i%,$(realpath $(SFS_DIR)/fakeroot.sav)) mksquashfs $^ $@ -noappend -e $(SFS_EXCLUDE)

%.sfs:	%/
	fakeroot $(patsubst %,-i%,$(realpath $(dir $@)fakeroot.sav)) mksquashfs $^ $@ -noappend -e $(SFS_EXCLUDE)
	
dbg:
	@echo $(sort $(MODS))
