#!/bin/bash

run () {
	RUN_COMMAND=$@
	echo -e "\033[1;33mTEST\033[0m $RUN_COMMAND"
	ERRORS=$($RUN_COMMAND 2>&1>/tmp/uwu_smoketest.out)
	RESULT=$?
	if test $RESULT -ne 0; then
		cat /tmp/uwu_smoketest.out
		echo -e "$ERRORS"
		echo -e "\033[1;5;41;37mFAIL\033[0m exit ($RESULT): $RUN_COMMAND"
		exit $RESULT
	else
		echo -ne "\033[1F"
		echo -e "\033[1;32mPASS\033[0m $RUN_COMMAND"
		echo -e "\033[1;32mPASS\033[0m $RUN_COMMAND"
	fi
}

conduwuit () {
	UWU_OPTS=$@
	rm -rf /tmp/uwu_smoketest.db
	echo -e "[global]\nserver_name = \"localhost\"\ndatabase_path = \"/tmp/uwu_smoketest.db\"" > /tmp/uwu_smoketest.toml
	cargo run $UWU_OPTS -- -c /tmp/uwu_smoketest.toml &
	sleep 5s
	kill -QUIT %1
	wait %1
	return $?
}

element () {
	ELEMENT_OPTS=$@
	run cargo check $ELEMENT_OPTS --all-targets
	run cargo clippy $ELEMENT_OPTS --all-targets -- -D warnings
	run cargo build $ELEMENT_OPTS --all-targets
	run cargo test $ELEMENT_OPTS --all-targets
	run cargo bench $ELEMENT_OPTS --all-targets
	run cargo run $ELEMENT_OPTS --bin conduit -- -V
	run conduwuit $ELEMENT_OPTS --bin conduit
}

vector () {
	VECTOR_OPTS=$@
	element $VECTOR_OPTS --no-default-features --features="rocksdb"
	element $VECTOR_OPTS --features=default
	element $VECTOR_OPTS --all-features
}

matrix () {
	run cargo fmt --all --check
	vector --profile=dev
	vector --profile=release
}

matrix &&
exit 0
