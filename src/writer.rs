//! Write las points.
//!
//! A `Writer` uses a `Header` for its configuration:
//!
//! ```
//! use std::io::Cursor;
//! use las::{Writer, Header};
//! let mut header = Header::from((1, 4));
//! let writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
//! ```
//!
//! The set of optional fields on the point format and the points must match exactly:
//!
//! ```
//! use std::io::Cursor;
//! use las::{Builder, Writer, Point};
//! use las::point::Format;
//! use las::Color;
//!
//! let mut builder = Builder::default();
//! builder.point_format = Format::new(1).unwrap();
//! let mut writer = Writer::new(Cursor::new(Vec::new()), builder.into_header().unwrap()).unwrap();
//!
//! let mut point = Point::default(); // default points don't have any optional attributes
//! assert!(writer.write(point.clone()).is_err());
//!
//! point.gps_time = Some(42.); // point format 1 requires gps time
//! writer.write(point.clone()).unwrap();
//!
//! point.color = Some(Color::new(1, 2, 3));
//! assert!(writer.write(point).is_err()); // the point's color would be lost
//! ```

use {Header, Point, Result};
use point::Format;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

quick_error! {
    /// Writer errors.
    #[derive(Debug)]
    pub enum Error {
        /// The writer is closed.
        Closed {
            description("the writer is closed")
        }
        /// The attributes of the point format and point do not match.
        PointAttributes(format: Format, point: Point) {
            description("the attributes of the point format and point do not match")
            display("the attributes of point format {:?} does not match point {:?}", format, point)
        }
    }
}

/// Writes LAS data.
///
/// The LAS header needs to be re-written when the writer closes. For convenience, this is done via
/// the `Drop` implementation of the writer. One consequence is that if the header re-write fails
/// during the drop, a panic will result. If you want to check for errors instead of panicing, use
/// `close` explicitly.
///
/// ```
/// use std::io::Cursor;
/// use las::Writer;
/// {
///     let mut writer = Writer::default();
///     writer.close().unwrap();
/// } // <- `close` is not called
/// ```
#[derive(Debug)]
pub struct Writer<W: Seek + Write> {
    closed: bool,
    header: Header,
    write: W,
}

impl<W: Seek + Write> Writer<W> {
    /// Creates a new writer.
    ///
    /// The header that is passed in will have various fields zero'd, e.g. bounds, number of
    /// points, etc.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    /// use las::Writer;
    /// let writer = Writer::new(Cursor::new(Vec::new()), Default::default());
    /// ```
    pub fn new(mut write: W, mut header: Header) -> Result<Writer<W>> {
        header.clear();
        header.clone().into_raw().and_then(|raw_header| {
            raw_header.write_to(&mut write)
        })?;
        for vlr in header.vlrs() {
            (*vlr).clone().into_raw().and_then(|raw_vlr| {
                raw_vlr.write_to(&mut write)
            })?;
        }
        if !header.vlr_padding().is_empty() {
            write.write_all(&header.vlr_padding())?;
        }
        Ok(Writer {
            closed: false,
            header: header,
            write: write,
        })
    }

    /// Writes a point.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    /// use las::Writer;
    ///
    /// let mut writer = Writer::default();
    /// writer.write(Default::default()).unwrap();
    /// ```
    pub fn write(&mut self, point: Point) -> Result<()> {
        if self.closed {
            return Err(Error::Closed.into());
        }
        if !point.matches(self.header.point_format()) {
            return Err(
                Error::PointAttributes(self.header.point_format(), point).into(),
            );
        }
        self.header.add_point(&point);
        point.into_raw(self.header.transforms()).and_then(
            |raw_point| {
                raw_point.write_to(&mut self.write, self.header.point_format())
            },
        )?;
        Ok(())
    }

    /// Close this writer.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    /// use las::Writer;
    /// let mut writer = Writer::default();
    /// writer.close().unwrap();
    /// assert!(writer.close().is_err());
    /// ```
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Err(Error::Closed.into());
        }
        for raw_evlr in self.header.evlrs().into_iter().map(|evlr| {
            evlr.clone().into_raw()
        })
        {
            raw_evlr?.write_to(&mut self.write)?;
        }
        // TODO support writers that aren't at the beginning of their write
        self.write.seek(SeekFrom::Start(0))?;
        self.header.clone().into_raw().and_then(|raw_header| {
            raw_header.write_to(&mut self.write)
        })?;
        self.closed = true;
        Ok(())
    }
}

impl<W: Write + Seek + Clone> Writer<W> {
    /// Closes this writer and returns its inner `Write`, seeked to the beginning of the las data.
    ///
    /// # Examples
    ///
    /// ```
    /// use las::Writer;
    /// let writer = Writer::default();
    /// let cursor = writer.into_inner().unwrap();
    /// ```
    pub fn into_inner(mut self) -> Result<W> {
        if !self.closed {
            self.close()?;
        }
        let mut write = self.write.clone();
        // TODO writers that aren't at the beginning of their write
        write.seek(SeekFrom::Start(0))?;
        Ok(write)
    }
}

impl Writer<BufWriter<File>> {
    /// Creates a new writer for a path.
    ///
    /// # Examples
    ///
    /// ```
    /// use las::Writer;
    /// let writer = Writer::from_path("/dev/null", Default::default());
    /// ```
    pub fn from_path<P: AsRef<Path>>(path: P, header: Header) -> Result<Writer<BufWriter<File>>> {
        File::create(path).map_err(::Error::from).and_then(|file| {
            Writer::new(BufWriter::new(file), header)
        })
    }
}

impl Default for Writer<Cursor<Vec<u8>>> {
    fn default() -> Writer<Cursor<Vec<u8>>> {
        Writer::new(Cursor::new(Vec::new()), Header::default()).unwrap()
    }
}

impl<W: Seek + Write> Drop for Writer<W> {
    fn drop(&mut self) {
        if !self.closed {
            self.close().expect("Error when dropping the writer");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Version;
    use byteorder::{LittleEndian, ReadBytesExt};
    use header::Builder;
    use point::Format;
    use std::io::Cursor;

    fn writer(format: Format, version: Version) -> Writer<Cursor<Vec<u8>>> {
        let mut builder = Builder::default();
        builder.point_format = format;
        builder.version = version;
        Writer::new(Cursor::new(Vec::new()), builder.into_header().unwrap()).unwrap()
    }

    #[test]
    fn las_1_0_point_data_start_signature() {
        let mut builder = Builder::default();
        builder.version = (1, 0).into();
        builder.vlrs.push(Default::default());
        let mut writer = Writer::new(Cursor::new(Vec::new()), builder.into_header().unwrap())
            .unwrap();
        writer.write(Default::default()).unwrap();
        let mut cursor = writer.into_inner().unwrap();
        cursor.set_position(281);
        assert_eq!(0xCCDD, cursor.read_u16::<LittleEndian>().unwrap());
    }

    #[test]
    fn already_closed() {
        let mut writer = Writer::default();
        writer.close().unwrap();
        assert!(writer.close().is_err());
        assert!(writer.write(Default::default()).is_err());
    }

    #[test]
    fn missing_extra_bytes() {
        let format = Format {
            extra_bytes: 1,
            ..Default::default()
        };
        let mut writer = writer(format, Version::new(1, 4));
        assert!(writer.write(Default::default()).is_err());
    }

    #[test]
    fn missing_gps_time() {
        let format = Format::new(1).unwrap();
        let mut writer = writer(format, Version::new(1, 2));
        assert!(writer.write(Default::default()).is_err());
    }

    #[test]
    fn missing_color() {
        let format = Format::new(2).unwrap();
        let mut writer = writer(format, Version::new(1, 2));
        assert!(writer.write(Default::default()).is_err());
    }

    #[test]
    fn missing_nir() {
        let format = Format::new(8).unwrap();
        let mut writer = writer(format, Version::new(1, 4));
        let point = Point {
            gps_time: Some(0.),
            color: Some(Default::default()),
            ..Default::default()
        };
        assert!(writer.write(point).is_err());
    }

    #[test]
    fn missing_waveform() {
        let format = Format::new(4).unwrap();
        let mut writer = writer(format, Version::new(1, 4));
        assert!(writer.write(Default::default()).is_err());
    }
}
