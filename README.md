# ramdisk

Rust rewrite of the initramfs ramdisk.

## The `root=` format

The `root=` kernel command line parameter (parsed in `Config::parse_cmdline_token`)
selects the root filesystem stack of the ramdisk. It is a `+`-separated list of
parts, for example:

```
root=/dev/vda1:test.sfs+mem
root=mem:/path/to/base.sfs+:/path/to/kernel.sfs+mem
root=smb://user@host/share:./*.sfs+:./\$arch/*kernel-\$kver.sfs+mem
```

### Mount procedure

During the `mountroot` stage all parts of the `root=` definition are mounted
individually, each one to its own mount point under `/.parts/`, and only after
that are they mounted together as an `overlay` filesystem on `/realroot`, with
the last part as the overlay `lowerdir`.

- If the last part is `mem`, it is mounted as `tmpfs`,
  making root filesystem changes to be written memory only.
- If part is a file, it is mounted with the `-o loop` option.
  If it has `.sfs` extension, then it is mounted as 'squashfs` filesystem.
- If a part contains `:`, then the part before `:` is handled as source
  for the mounted part(s), mounted to `/live`, and the string after `:`
  designates file in that source, which will be mounted  via the `-o loop`
  option. If the part before `:` is an empty string, the previously
  mounted `/live` mount is re-used. A non-empty string makes the source
  device for the loop-mounted file `/live1`, the next one `/live2`, and
  so on. All `/live*` mounts are move-mounted to `/realroot/liveXX` after
  the overlay mount.

In case of overlayfs the workdir and upperdir have to be in the same
filesystem, therefore if the last part is `+mem`, then upperdir and workdir
are subdirectories under the newly mounted tmpfs part.

The last part can also be a real device or a loop file with some other
filesystem (like ext4) instead of `+mem`. In that case the root filesystem
changes are saved to permanent storage. The same overlay upperdir/workdir rules
apply, and the last component has to be mounted read-write (like with `mem`).
Therefore the last part cannot be squashfs, but could be some other filesystem
(like ext4) loopfile or device. If it is a loopfile, then source of that loopfile
has to be mounted as read-write as well.

### Copy up

If the rootfs is prefixed with `mem:`, then `/live` should be `tmpfs` and all
the normally loop-mounted components (e.g. `.sfs` files) files should be copied there
before them to `/.parts`.

### Final layout

After mounting as overlay, `/realroot/.parts` is created and every individual
part is moved from `/.parts/XXX` to `/realroot/.parts/XXX`.

`/live*` directories are moved to the `/realroot/live*`.
