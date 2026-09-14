if [ ! -d target ]; then
    mkdir target
    uid=$(dumpsys package com.termux | grep appId | awk 'NR==1{print $1}' | cut -d '=' -f2)
    chown -R $uid:$uid ./target
    chmod -R 0755 ./target
fi
name=$(basename $(dirname "$0"))
{
    for i in $(find crates -name "*.rs") $(find crates -name "Cargo.toml")  $(find app -name "*.rs") app/Cargo.toml Cargo.toml; do
        echo "这是$i: "
        cat $i
        echo "\n--------------\n"
    done
} > target/"$name"_all_code.txt
