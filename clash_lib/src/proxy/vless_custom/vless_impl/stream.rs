use std::{fmt::Debug, pin::Pin, task::Poll};

use bytes::{Buf,  BytesMut};
use futures::ready;

use crate::session::SocksAddr;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub struct VlessStream<S> {
    stream: S,
    dst: SocksAddr,
    auth: Uuid,
    is_udp: bool,

    read_state: ReadState,
    read_pos: usize,
    read_buf: BytesMut,

    write_state: WriteState,
    write_buf: BytesMut,
}

impl<S> Debug for VlessStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VlessStream")
            .field("dst", &self.dst)
            .field("is_udp", &self.is_udp)
            .finish()
    }
}

enum ReadState {
    ReadVersion,
    ReadAdditionalInformationLength,
    ReadAdditionalInformation(u8),
    ReadUdpDataLength,
    ReadUdpData(u16),
    ReadTcpData,
    StreamFlushingData(usize),
}

enum WriteState {
    BuildingData,
    FlushingData(usize, (usize, usize)),
}

use crate::common::io::{ReadExactBase, ReadExt};

impl<S: AsyncRead + Unpin> ReadExactBase for VlessStream<S> {
    type I = S;

    fn decompose(&mut self) -> (&mut Self::I, &mut BytesMut, &mut usize) {
        (&mut self.stream, &mut self.read_buf, &mut self.read_pos)
    }
}

impl<S> VlessStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub(crate) async fn new(
        stream: S,
        uuid: &Uuid,
        dst: &SocksAddr,
        is_udp: bool,
    ) -> std::io::Result<VlessStream<S>> {
        let mut stream = Self {
            stream,
            dst: dst.clone(),
            auth: uuid.clone(),
            is_udp,

            read_state: ReadState::ReadVersion,
            read_pos: 0,
            read_buf: BytesMut::new(),

            write_state: WriteState::BuildingData,
            write_buf: BytesMut::new(),
        };

        stream.send_header_request().await?;

        Ok(stream)
    }
}

impl<S> VlessStream<S>
where
    S: AsyncWrite + Unpin,
{
    async fn send_header_request(&mut self) -> std::io::Result<()> {
        let version = [0];
        let additional_information_length = vec![0];
        let instruction = [if self.is_udp { 2 } else { 1 }];
        let mut header_bytes = Vec::new();
        header_bytes.extend_from_slice(&version);
        header_bytes.extend_from_slice(self.auth.as_bytes().as_slice());
        header_bytes.extend_from_slice(additional_information_length.as_slice());
        header_bytes.extend_from_slice(&instruction);
        self.dst.write_to_buf_vless(&mut header_bytes);

        self.stream.write_all(&header_bytes).await?;
        self.stream.flush().await?;

        Ok(())
    }
}

impl<S> AsyncRead for VlessStream<S>
where
    S: AsyncRead + Unpin + Send,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            match self.read_state {
                ReadState::ReadVersion => {
                    let this = &mut *self;
                    ready!(this.poll_read_exact(cx, 1))?;
                    let version = this.read_buf.split_to(1).get_u8();
                    assert_eq!(0, version);
                    self.read_state = ReadState::ReadAdditionalInformationLength;
                }
                ReadState::ReadAdditionalInformationLength => {
                    let this = &mut *self;
                    ready!(this.poll_read_exact(cx, 1))?;
                    let mut buf = this.read_buf.split_to(1);
                    let count = buf.get_u8();
                    if count > 0 {
                        self.read_state =
                            ReadState::ReadAdditionalInformation(count);
                    } else {
                        if self.is_udp {
                            self.read_state = ReadState::ReadUdpDataLength;
                        } else {
                            self.read_state = ReadState::ReadTcpData;
                        }
                    }
                }

                ReadState::ReadAdditionalInformation(length) => {
                    let this = &mut *self;
                    let length = length as usize;
                    ready!(this.poll_read_exact(cx, length))?;
                    let _ = this.read_buf.split_to(length);
                    if self.is_udp {
                        self.read_state = ReadState::ReadUdpDataLength;
                    } else {
                        self.read_state = ReadState::ReadTcpData;
                    }
                }
                ReadState::ReadUdpDataLength => {
                    let this = &mut *self;
                    ready!(this.poll_read_exact(cx, 2))?;
                    let mut buf = this.read_buf.split_to(2);
                    let count = buf.get_u16();
                    self.read_state = ReadState::ReadUdpData(count);
                }
                ReadState::ReadUdpData(length) => {
                    let this = &mut *self;
                    let length = length as usize;
                    ready!(this.poll_read_exact(cx, length))?;
                    this.read_state = ReadState::StreamFlushingData(this.read_buf.len());
                }
                ReadState::ReadTcpData => {
                    let this = &mut *self;
                    ready!(this.poll_read_once(cx, buf.remaining()))?;
                    this.read_state = ReadState::StreamFlushingData(this.read_buf.len());
                }
                ReadState::StreamFlushingData(size) => {
                    let to_read = std::cmp::min(buf.remaining(), size);
                    let payload = self.read_buf.split_to(to_read);
                    buf.put_slice(&payload);
                    if to_read < size {
                        // there're unread data, continues in next poll
                        self.read_state =
                            ReadState::StreamFlushingData(size - to_read);
                    } else {
                        if self.is_udp {
                            self.read_state = ReadState::ReadUdpDataLength;
                        } else {
                            self.read_state = ReadState::ReadTcpData;
                        }
                    }

                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

impl<S> AsyncWrite for VlessStream<S>
where
    S: AsyncWrite + Unpin + Send,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        loop {
            match self.write_state {
                WriteState::BuildingData => {
                    let this = &mut *self;
                    let consume_len = buf.len();
                    if this.is_udp {
                        let len_buf = (buf.len() as u16).to_be_bytes();
                        let mut new_buf = Vec::new();
                        new_buf.extend_from_slice(&len_buf);
                        new_buf.extend_from_slice(&buf);
                        this.write_buf.extend_from_slice(new_buf.as_slice());
                    } else {
                        this.write_buf.extend_from_slice(buf);
                    }
                    // ready to write data
                    self.write_state = WriteState::FlushingData(
                        consume_len,
                        (this.write_buf.len(), 0),
                    );
                }

                // consumed is the consumed plaintext length we're going to
                // return to caller. total is total length of
                // the ciphertext data chunk we're going to write to remote.
                // written is the number of ciphertext bytes were written.
                WriteState::FlushingData(consumed, (total, written)) => {
                    let this = &mut *self;

                    // There would be trouble if the caller change the buf upon
                    // pending, but I believe that's not a
                    // usual use case.
                    let nw = ready!(tokio_util::io::poll_write_buf(
                        Pin::new(&mut this.stream),
                        cx,
                        &mut this.write_buf
                    ))?;
                    if nw == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::WriteZero,
                            "failed to write whole data",
                        ))
                            .into();
                    }

                    if written + nw >= total {
                        // data chunk written, go to next chunk
                        this.write_state = WriteState::BuildingData;
                        return Poll::Ready(Ok(consumed));
                    }

                    this.write_state =
                        WriteState::FlushingData(consumed, (total, written + nw));
                }
            }
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let Self { stream, .. } = self.get_mut();
        Pin::new(stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let Self { stream, .. } = self.get_mut();
        Pin::new(stream).poll_shutdown(cx)
    }
}
