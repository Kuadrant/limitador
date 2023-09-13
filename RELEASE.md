# How to release Limitador

## Process

### `limitador` crate to crates.io

 - Create a local branch, whatever the name, e.g. `release`

```sh
git checkout -b release
```

 - Remove the `-dev` suffix from both `Cargo.toml`

 ```diff
diff --git a/limitador-server/Cargo.toml b/limitador-server/Cargo.toml
index dd2f311..b555df8 100644
--- a/limitador-server/Cargo.toml
+++ b/limitador-server/Cargo.toml
@@ -1,6 +1,6 @@
 [package]
 name = "limitador-server"
-version = "1.3.0-dev"
+version = "1.3.0"
diff --git a/limitador/Cargo.toml b/limitador/Cargo.toml
index 3aebf9d..d17b92b 100644
--- a/limitador/Cargo.toml
+++ b/limitador/Cargo.toml
@@ -1,6 +1,6 @@
 [package]
 name = "limitador"
-version = "0.5.0-dev"
+version = "0.5.0"
 ```

 - Commit the changes

```sh
git commit -am "[release] Releasing Crate 0.5.0 and Server 1.3.0"
```

 - Create a tag named after the version, with the `crate-v` prefix 

```sh
git tag -a crate-v0.5.0 -m "[tag] Limitador crate v0.5.0"
```

 - Push the tag to remote

```sh
git push origin crate-v0.5.0
```

 - Manually run the `Release crate` workflow action on [Github](https://github.com/Kuadrant/limitador/actions/workflows/release.yaml) providing the version to release in the input box, e.g. `0.5.0`, if all is correct, this should push the release to [crates.io](https://crates.io/crates/limitador/versions)
 - Create the release and release notes on [Github](https://github.com/Kuadrant/limitador/releases/new) using the tag from above, named: `Limitador crate vM.m.d`


### `limitador-server` container image to quay.io

 - Create a branch for your version with the `v` prefix, e.g. `v1.3.0`
 - Make sure your `Cargo.toml` is reflecting the proper version, see above
 - Push the branch to remote, which should create a matching release to quay.io with the tag name based of your branch, i.e. in this case `v1.3.0`
 - Create a tag with the `server-v` prefix, e.g. `server-v1.3.0`
 - Push the tag to Github
 - Create the release and release notes on [Github](https://github.com/Kuadrant/limitador/releases/new) using the tag from above, named: `vM.m.d`
 - Delete the branch, only keep the tag used for the release

## After the release

 - Create a `next` branch off `main` 
 - Update the _both_ Cargo.toml to point to the next `-dev` release
 - Create PR
 - Merge to `main`

 ```diff
diff --git a/limitador-server/Cargo.toml b/limitador-server/Cargo.toml
index dd2f311..011a2cd 100644
--- a/limitador-server/Cargo.toml
+++ b/limitador-server/Cargo.toml
@@ -1,6 +1,6 @@
 [package]
 name = "limitador-server"
-version = "1.3.0-dev"
+version = "1.4.0-dev"
 ```

