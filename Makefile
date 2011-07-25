KVERS=$(shell uname -r)
MODDIR=/lib/modules/$(KVERS)

SHELL=/bin/bash
RAMDISK=ramdisk-$(KVERS)
ifndef RD_DIR
  RD_DIR:=$(shell mktemp -d /tmp/ramdisk.XXXXXX)
  export RD_DIR
endif

RD_TMPL=ramdisk.template
RD_FILES=$(shell find $(RD_TMPL) -not -type l)

findprog=$(shell find {/usr,}/{s,}bin -maxdepth 1 \( -false $(foreach name,$(1),-or -name "$(name)") \))

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

NET_DISABLED=arcnet/ phy/ appletalk/ tokenring/ wan/ wireless/ pcmcia/ hamradio/ irda/ wlan ppp ath
NETDRV=$(basename $(notdir $(shell find $(MODDIR)/kernel/drivers/net -name '*.ko' $(patsubst %,-not -path '*/%*',$(NET_DISABLED))))) cifs nfs $(basename $(shell find $(MODDIR) -name md4.ko))

USBMODS=sd-mod usb-storage uhci-hcd ehci-hcd ohci-hcd usbhid
DISKDRV=$(basename $(notdir $(shell find $(MODDIR)/kernel/drivers/{ata,scsi,ide} -name '*.ko'))) cciss mptspi mptsas mmc_block sdhci_pci

CRYPTOMODS=dm-crypt $(basename $(notdir $(shell find $(MODDIR)/kernel -name cbc.ko $(patsubst %,-or -name '%*.ko',aes sha))))
CRYPTOPROGS=cryptsetup

FSMODS=ext2 ext3 ext4 xfs ntfs vfat reiserfs isofs loop
OTHERMODS=aufs squashfs

FUSEPROGS=/sbin/mount.fuse
FUSEMODS=fuse

NTFSPROGS=$(call findprog,ntfs* scrounge-ntfs)

UDEVPROGS=udev{d,adm} $(wildcard /lib/libnss_files.so.* /lib/i386-linux-gnu/libnss_files.so.*) /sbin/{dmsetup,blkid} $(shell find $$(dpkg -L udev  | grep '^/lib/udev/[^/]*$$') -maxdepth 0 -type f)
UDEVFILES=$(shell dpkg -L udev | grep -E '^/lib/udev/rules.d/') $(shell find /lib/udev/keymaps -type f)

NETPROGS=$(wildcard /lib/libnss_dns.so.* /lib/libnss_files.so.* /lib/i386-linux-gnu/libnss_files.so.* /lib/i386-linux-gnu/libnss_dns.so.*) $(call findprog,telnet udp-[rs]e*er *mount.cifs socat)

WIFIMODS=$(basename $(notdir $(shell find $(MODDIR)/kernel/drivers/net/wireless -name '*.ko')))
WIFIPROGS=iwlist iwconfig

DISKMODS=$(DISKDRV) nls_cp437 nls_iso8859-1 nls_utf8 cdrom i2c_i801
MODS=$(DISKMODS) $(FSMODS) $(OTHERMODS) $(USBMODS)
NORMMODS=yenta_socket $(CRYPTOMODS)

MINPROGS=modprobe /usr/lib/klibc/bin/* $(UDEVPROGS)
NORMPROGS=rmmod halt busybox losetup $(call findprog,fdisk lspci lvm kexec) $(CRYPTOPROGS)

TPM_PROGS=tcsd
TPM_MODS=$(basename $(shell find $(MODDIR) -name "tpm*.ko"))

PKCS11_PROGS=$(call findprog,openssl pcscd pkcs15-tool) /usr/lib/opensc-pkcs11.so /usr/lib/ssl/engines/engine_pkcs11.so $(find /usr/lib/pcsc/drivers -name "*.so")
PKCS11_FILES=/usr/lib/ssl/openssl.cnf /etc/opensc/opensc.conf

RELAXMODS=fuse cdrom ehci-hcd loop ohci-hcd uhci-hcd aufs ext2 ext3 ext4 sd-mod yenta_socket reiserfs sdhci_pci

DATAFILES=$(UDEVFILES)

ifeq ($(MODS),dep)
MODS=$(MODS_RUNNING)
endif

ifeq ($(TGT),vmware)
NETDRV=pcnet32
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
MIN=1
NET=1
endif

ifdef NTFS
PROGS+=$(NTFSPROGS) 
FUSE=1
endif

ifdef FUSE
MODS+=$(FUSEMODS) 
PROGS+=$(FUSEPROGS)
endif

ifdef NET
PROGS+=$(NETPROGS)
MODS+=$(NETDRV) 
FSMODS+=nfs
endif

ifdef TPM
PROGS+=$(TPM_PROGS)
MODS+=$(TPM_MODS)
endif

ifdef PKCS11
PROGS+=$(PKCS11_PROGS)
DATAFILES+=$(PKCS11_FILES)
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

KVM_APPEND=root=none quiet


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
	kvm -kernel /boot/vmlinuz-$(KVERS) -initrd $(RAMDISK) -append "$(KVM_APPEND)"

nfsroot:	clean
	$(MAKE) $(MAKEFLAGS) TGT=nfsroot
	cp $(RAMDISK) netinstall/tftp

$(RAMDISK): $(RD_FILES) Makefile
	cp -T -r $(RD_TMPL) $(RD_DIR)
	./copy_exe -m $(RD_DIR) $(PROGS)
	test -z "$(DATADIRS)" || cp --parents -r $(DATADIRS) $(RD_DIR)
	test -z "$(DATAFILES)" || cp --parents $(DATAFILES) $(RD_DIR)
	test ! -e "$(RD_DIR)/sbin/udevadm" -o -e "$(RD_DIR)/sbin/udevsettle" || ln -s udevadm "$(RD_DIR)/sbin/udevsettle"
	test ! -e "$(RD_DIR)/usr/lib/klibc/bin/sh.shared" -o -e "$(RD_DIR)/bin/sh" || ln -s ../usr/lib/klibc/bin/sh.shared "$(RD_DIR)/bin/sh"
	test ! -e "$(RD_DIR)/bin/sh" -o -e "$(RD_DIR)/bin/bash" || ln -s sh "$(RD_DIR)/bin/bash"
	test -z "$(MODS_PRELOAD)" || for mod in $(MODS_PRELOAD);do echo "$$mod" ; done > "$(RD_DIR)/etc/modules.preload"
	grep -h -o "GROUP=[^ ]*" "$(RD_DIR)/lib/udev/rules.d"/*.rules | sed -e 's/GROUP="\([^"]*\)".*/^\1:/' | sort -u | grep -f - /etc/group | cut -f1-3 -d: | sed -e 's/$$/:/' >"$(RD_DIR)/etc/group"
	./moddep $(RD_DIR) -r "$(RELAXMODS)" $(sort $(basename $(notdir $(MODS))))
	echo "$$MAKEFLAGS" >"$(RD_DIR)/.makeflags"
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
