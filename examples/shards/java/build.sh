#!/bin/sh
# brok installs one file: a shell line that starts Java, followed by the jar.
# Add your dependencies to the jar (or build a "fat" jar with Maven or Gradle).
set -e
rm -rf out dist
mkdir -p out dist
javac --release 17 -d out Main.java Json.java
jar --create --file out/shard.jar --main-class Main -C out .
{
    printf '#!/bin/sh\nexec java -Xmx64m -jar "$0" "$@"\n'
    cat out/shard.jar
} > dist/todo-java
chmod +x dist/todo-java
