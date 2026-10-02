#!/bin/sh
# Starts the Docker services the opt-in BDD tags need (see README.md,
# "Environment"). Idempotent: a container that already exists is started,
# not recreated. Remove one with `docker rm -f <name>` to recreate it.
#
#   tests/bdd/services.sh            # all services
#   tests/bdd/services.sh ftp mysql  # only these
set -eu

run() {
    name=$1
    shift
    if docker container inspect "$name" >/dev/null 2>&1; then
        docker start "$name" >/dev/null
    else
        docker run -d --name "$name" "$@" >/dev/null
    fi
    echo "$name up"
}

postgres() { run r8r-bdd-postgres -p 127.0.0.1:5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16; }
redis() { run r8r-bdd-redis -p 127.0.0.1:6379:6379 redis:7; }
mailpit() { run r8r-bdd-mailpit -p 127.0.0.1:1025:1025 -p 127.0.0.1:8025:8025 axllent/mailpit; }
mysql() { run r8r-bdd-mysql -p 127.0.0.1:3306:3306 -e MYSQL_ROOT_PASSWORD=mysql -e MYSQL_DATABASE=r8r mysql:8.4; }
mongo() { run r8r-bdd-mongo -p 127.0.0.1:27017:27017 mongo:7; }
ssh() {
    run r8r-bdd-ssh -p 127.0.0.1:2222:2222 -e PUID=1000 -e PGID=1000 -e USER_NAME=r8r -e USER_PASSWORD=r8r \
        -e PASSWORD_ACCESS=true lscr.io/linuxserver/openssh-server:latest
}
# vsftpd, not pure-ftpd: pure-ftpd's password hashing makes each login take
# ~2 s and logins queue up, so parallel scenarios hang until the 30 s CLI
# timeout. The config file must come before the -o overrides (later wins).
ftp() {
    run r8r-bdd-ftp -p 127.0.0.1:2121:21 -p 127.0.0.1:30000-30049:30000-30049 -e "USERS=r8r|r8r|/home/r8r" \
        delfer/alpine-ftp-server vsftpd /etc/vsftpd/vsftpd.conf -obackground=NO \
        -opasv_min_port=30000 -opasv_max_port=30049 -opasv_address=127.0.0.1 -opasv_addr_resolve=NO \
        -ochroot_local_user=YES -oallow_writeable_chroot=YES -omax_per_ip=0 -omax_clients=0
}
mssql() {
    run r8r-bdd-mssql -p 127.0.0.1:1433:1433 -e ACCEPT_EULA=Y -e 'MSSQL_SA_PASSWORD=R8r_Passw0rd!' \
        mcr.microsoft.com/mssql/server:2022-latest
    # The `r8r` database the scenarios use; retried while SQL Server starts.
    for _ in $(seq 1 60); do
        docker exec r8r-bdd-mssql /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P 'R8r_Passw0rd!' -C \
            -Q "IF DB_ID('r8r') IS NULL CREATE DATABASE r8r" >/dev/null 2>&1 && return
        sleep 2
    done
    echo "r8r-bdd-mssql: database r8r not created" >&2
    return 1
}

rabbitmq() { run r8r-bdd-rabbitmq -p 127.0.0.1:5672:5672 -p 127.0.0.1:15672:15672 rabbitmq:3-management; }
mqtt() {
    run r8r-bdd-mqtt -p 127.0.0.1:1883:1883 eclipse-mosquitto:2 \
        sh -c 'printf "listener 1883\nallow_anonymous true\n" > /tmp/m.conf && exec mosquitto -c /tmp/m.conf'
}
# Single-node KRaft broker, advertised on 127.0.0.1:9092.
kafka() { run r8r-bdd-kafka -p 127.0.0.1:9092:9092 apache/kafka:3.8.0; }

if [ $# -eq 0 ]; then
    set -- postgres redis mailpit mysql mongo ssh ftp mssql rabbitmq mqtt kafka
fi
for service in "$@"; do
    "$service"
done
